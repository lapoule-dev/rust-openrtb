//! Demo SSP / ad server: replays randomized fixture bid requests to one or
//! more bidders, runs the auction, fires the winners' nurl/burl and prints a
//! report.

mod auction;
mod traffic;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use buffa::Message;
use clap::{Parser, ValueEnum};
use futures_util::{StreamExt, future::join_all, stream};
use openrtb_model::OpenRtbJson;
use openrtb_model::v2::{BidRequest, BidResponse};
use reqwest::header::CONTENT_TYPE;
use tracing_subscriber::EnvFilter;

use auction::{AuctionType, Win};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum WireFormat {
    Json,
    Protobuf,
}

impl WireFormat {
    fn content_type(self) -> &'static str {
        match self {
            WireFormat::Json => "application/json",
            WireFormat::Protobuf => "application/x-protobuf",
        }
    }
}

/// Demo SSP: sends randomized OpenRTB 2.6 requests to bidders and runs the auction.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Bidder endpoint; repeat for several bidders.
    #[arg(
        short,
        long = "bidder",
        env = "SSP_BIDDERS",
        value_delimiter = ',',
        default_value = "http://127.0.0.1:8080/openrtb2/bid"
    )]
    bidders: Vec<String>,
    /// Number of auctions to run.
    #[arg(short = 'n', long, default_value_t = 1000)]
    requests: usize,
    /// Auctions in flight at once.
    #[arg(short, long, default_value_t = 32)]
    concurrency: usize,
    /// Wire format of requests (responses are read by their Content-Type).
    #[arg(short, long, value_enum, default_value_t = WireFormat::Json)]
    format: WireFormat,
    /// Auction type.
    #[arg(short, long, value_enum, default_value_t = AuctionType::First)]
    auction: AuctionType,
    /// Override tmax (ms) of every request; default: the fixture's, or 120.
    #[arg(long)]
    tmax: Option<i32>,
    /// Directory holding iab/ and scala/ fixture bid requests.
    #[arg(long, default_value = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures"))]
    fixtures: PathBuf,
    /// RNG seed, for reproducible traffic.
    #[arg(long)]
    seed: Option<u64>,
    /// Don't call the winners' nurl/burl.
    #[arg(long)]
    no_notify: bool,
    /// Print one line per auction.
    #[arg(short, long)]
    verbose: bool,
}

struct Ctx {
    client: reqwest::Client,
    bidders: Vec<String>,
    format: WireFormat,
    auction: AuctionType,
    notify: bool,
    verbose: bool,
}

#[derive(Debug)]
enum Reply {
    NoBid,
    Bids(BidResponse),
    Timeout,
    Error(String),
}

/// Everything one auction produced, folded into the report afterwards.
struct Outcome {
    template: usize,
    /// Per bidder: latency and reply kind.
    replies: Vec<(Duration, ReplyKind)>,
    wins: Vec<Win>,
    dropped_bids: usize,
    notices_ok: usize,
    notices_failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyKind {
    Bid,
    NoBid,
    Timeout,
    Error,
    Invalid,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let templates = traffic::load_templates(&args.fixtures)?;
    if templates.is_empty() {
        return Err(format!("no *bidrequest*.json under {}", args.fixtures.display()).into());
    }
    let mut rng = args
        .seed
        .map_or_else(fastrand::Rng::new, fastrand::Rng::with_seed);
    // Generate the traffic up front so that the measured loop only does RTB work.
    let traffic: Vec<(usize, BidRequest)> = (0..args.requests)
        .map(|_| {
            let t = rng.usize(..templates.len());
            (t, traffic::randomize(&mut rng, &templates[t].1, args.tmax))
        })
        .collect();

    let ctx = Arc::new(Ctx {
        client: reqwest::Client::builder()
            .pool_max_idle_per_host(args.concurrency)
            .tcp_nodelay(true)
            .build()?,
        bidders: args.bidders.clone(),
        format: args.format,
        auction: args.auction,
        notify: !args.no_notify,
        verbose: args.verbose,
    });

    println!(
        "demo-ssp: {} auctions, {} templates, {} bidder(s), wire={:?}, auction={:?}-price, concurrency={}",
        args.requests,
        templates.len(),
        ctx.bidders.len(),
        args.format,
        args.auction,
        args.concurrency
    );
    let start = Instant::now();
    let outcomes: Vec<Outcome> = stream::iter(traffic)
        .map(|(t, req)| run_auction(ctx.clone(), t, req))
        .buffer_unordered(args.concurrency.max(1))
        .collect()
        .await;
    let elapsed = start.elapsed();

    report(&ctx, &templates, &outcomes, elapsed);
    Ok(())
}

async fn call_bidder(ctx: &Ctx, url: &str, body: Vec<u8>, tmax: Duration) -> Reply {
    let res = ctx
        .client
        .post(url)
        .header(CONTENT_TYPE, ctx.format.content_type())
        .header("x-openrtb-version", "2.6")
        .timeout(tmax)
        .body(body)
        .send()
        .await;
    let res = match res {
        Ok(r) => r,
        Err(e) if e.is_timeout() => return Reply::Timeout,
        Err(e) => return Reply::Error(e.to_string()),
    };
    match res.status().as_u16() {
        204 => return Reply::NoBid,
        200 => {}
        s => return Reply::Error(format!("HTTP {s}")),
    }
    let protobuf = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("protobuf") || ct.contains("octet-stream"));
    let bytes = match res.bytes().await {
        Ok(b) => b,
        Err(e) if e.is_timeout() => return Reply::Timeout,
        Err(e) => return Reply::Error(e.to_string()),
    };
    // An empty 200 is a no-bid too, for lenient bidders.
    if bytes.is_empty() {
        return Reply::NoBid;
    }
    let decoded = if protobuf {
        BidResponse::decode_from_slice(&bytes).map_err(|e| e.to_string())
    } else {
        BidResponse::from_json_slice(&bytes).map_err(|e| e.to_string())
    };
    match decoded {
        Ok(r) => Reply::Bids(r),
        Err(e) => Reply::Error(format!("undecodable response: {e}")),
    }
}

async fn notice(ctx: &Ctx, url: &str) -> bool {
    let res = ctx
        .client
        .get(url)
        .timeout(Duration::from_secs(1))
        .send()
        .await;
    match res {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => {
            tracing::warn!(url, status = %r.status(), "notice refused");
            false
        }
        Err(e) => {
            tracing::warn!(url, %e, "notice failed");
            false
        }
    }
}

async fn run_auction(ctx: Arc<Ctx>, template: usize, req: BidRequest) -> Outcome {
    let body = match ctx.format {
        WireFormat::Json => req.to_json_vec(),
        WireFormat::Protobuf => req.encode_to_vec(),
    };
    let tmax = Duration::from_millis(req.tmax.unwrap_or(120).max(1) as u64);

    // Fan out to every bidder concurrently, each bounded by tmax.
    let replies = join_all(ctx.bidders.iter().map(|url| {
        let body = body.clone();
        let ctx = &ctx;
        async move {
            let t0 = Instant::now();
            let reply = call_bidder(ctx, url, body, tmax).await;
            (t0.elapsed(), reply)
        }
    }))
    .await;

    let mut kinds = Vec::with_capacity(replies.len());
    let mut candidates = Vec::new();
    let mut dropped_bids = 0;
    for (i, (latency, reply)) in replies.into_iter().enumerate() {
        let kind = match reply {
            Reply::NoBid => ReplyKind::NoBid,
            Reply::Timeout => ReplyKind::Timeout,
            Reply::Error(e) => {
                tracing::warn!(bidder = %ctx.bidders[i], %e, "bidder error");
                ReplyKind::Error
            }
            Reply::Bids(resp) => match auction::validate(&req, &resp, i) {
                Ok((valid, dropped)) => {
                    dropped_bids += dropped;
                    let kind = if valid.is_empty() {
                        ReplyKind::NoBid
                    } else {
                        ReplyKind::Bid
                    };
                    candidates.extend(valid);
                    kind
                }
                Err(e) => {
                    tracing::warn!(bidder = %ctx.bidders[i], %e, "invalid response");
                    ReplyKind::Invalid
                }
            },
        };
        kinds.push((latency, kind));
    }

    let wins = auction::run(&req, candidates, ctx.auction);
    let (mut notices_ok, mut notices_failed) = (0, 0);
    for win in &wins {
        if ctx.verbose {
            println!(
                "auction {} imp {} -> bidder #{} crid {} bid {:.4} pays {:.4} {}",
                req.id.as_deref().unwrap_or(""),
                win.candidate.bid.impid.as_deref().unwrap_or(""),
                win.candidate.bidder,
                win.candidate.bid.crid.as_deref().unwrap_or(""),
                win.candidate.bid.price.unwrap_or(0.0),
                win.price,
                win.candidate.cur,
            );
        }
        if !ctx.notify {
            continue;
        }
        // nurl on win; burl once the ad renders, which this demo assumes.
        for url in [&win.candidate.bid.nurl, &win.candidate.bid.burl]
            .into_iter()
            .flatten()
        {
            if notice(&ctx, &auction::substitute(url, &req, win)).await {
                notices_ok += 1;
            } else {
                notices_failed += 1;
            }
        }
    }
    if ctx.verbose && wins.is_empty() {
        println!("auction {} -> no bid", req.id.as_deref().unwrap_or(""));
    }
    Outcome {
        template,
        replies: kinds,
        wins,
        dropped_bids,
        notices_ok,
        notices_failed,
    }
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let i = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * n as f64 / total as f64
    }
}

fn report(ctx: &Ctx, templates: &[(String, BidRequest)], outcomes: &[Outcome], elapsed: Duration) {
    let n = outcomes.len();
    println!(
        "\n{n} auctions in {:.2} s: {:.0} QPS",
        elapsed.as_secs_f64(),
        n as f64 / elapsed.as_secs_f64()
    );

    for (i, url) in ctx.bidders.iter().enumerate() {
        let mut lat: Vec<Duration> = outcomes.iter().map(|o| o.replies[i].0).collect();
        lat.sort();
        let count = |k: ReplyKind| outcomes.iter().filter(|o| o.replies[i].1 == k).count();
        let bids = count(ReplyKind::Bid);
        let won = outcomes
            .iter()
            .flat_map(|o| &o.wins)
            .filter(|w| w.candidate.bidder == i)
            .count();
        println!("\nbidder #{i} {url}");
        println!(
            "  bid rate {:.1}% ({bids} bids, {} no-bids, {} timeouts, {} errors, {} invalid), {won} imps won",
            pct(bids, n),
            count(ReplyKind::NoBid),
            count(ReplyKind::Timeout),
            count(ReplyKind::Error),
            count(ReplyKind::Invalid),
        );
        println!(
            "  latency p50 {}  p90 {}  p99 {}  max {}",
            ms(percentile(&lat, 0.50)),
            ms(percentile(&lat, 0.90)),
            ms(percentile(&lat, 0.99)),
            ms(lat.last().copied().unwrap_or_default()),
        );
    }

    let wins: Vec<&Win> = outcomes.iter().flat_map(|o| &o.wins).collect();
    let filled = outcomes.iter().filter(|o| !o.wins.is_empty()).count();
    println!(
        "\nfill: {filled}/{n} auctions ({:.1}%), {} imps won, {} bids dropped (under floor / unknown imp)",
        pct(filled, n),
        wins.len(),
        outcomes.iter().map(|o| o.dropped_bids).sum::<usize>(),
    );
    if !wins.is_empty() {
        let mut prices: Vec<f64> = wins.iter().map(|w| w.price_usd).collect();
        prices.sort_by(f64::total_cmp);
        let total: f64 = prices.iter().sum();
        println!(
            "clearing CPM (USD): min {:.4}  p50 {:.4}  avg {:.4}  max {:.4}; spend {:.4} USD",
            prices[0],
            prices[prices.len() / 2],
            total / prices.len() as f64,
            prices[prices.len() - 1],
            total / 1000.0,
        );
        let mut by_crid: BTreeMap<(&str, &str), (usize, f64)> = BTreeMap::new();
        for w in &wins {
            let e = by_crid
                .entry((
                    w.candidate.bid.crid.as_deref().unwrap_or("?"),
                    w.candidate.cur.as_str(),
                ))
                .or_default();
            e.0 += 1;
            e.1 += w.price;
        }
        println!("wins by creative:");
        for ((crid, cur), (count, sum)) in by_crid {
            println!(
                "  {crid:<22} {count:>6} wins  avg {:.4} {cur}",
                sum / count as f64
            );
        }
    }
    let mut by_template: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for o in outcomes {
        let e = by_template.entry(&templates[o.template].0).or_default();
        e.0 += 1;
        e.1 += usize::from(!o.wins.is_empty());
    }
    println!("fill by fixture:");
    for (name, (count, filled)) in by_template {
        println!(
            "  {name:<48} {filled:>6}/{count:<6} ({:.0}%)",
            pct(filled, count)
        );
    }
    if ctx.notify {
        println!(
            "notices (nurl + burl): {} ok, {} failed",
            outcomes.iter().map(|o| o.notices_ok).sum::<usize>(),
            outcomes.iter().map(|o| o.notices_failed).sum::<usize>(),
        );
    }
}
