use clap::Parser;
use demo_bidder::BidderConfig;
use tracing_subscriber::EnvFilter;

/// Demo OpenRTB 2.6 bidder: POST /openrtb2/bid (JSON or protobuf), GET /win, /bill, /stats.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Address to bind.
    #[arg(long, env = "BIDDER_HOST", default_value = "127.0.0.1")]
    host: String,
    /// Port to listen on.
    #[arg(short, long, env = "BIDDER_PORT", default_value_t = 8080)]
    port: u16,
    /// Base URL the exchange uses to reach this bidder (nurl/burl);
    /// defaults to http://<host>:<port>.
    #[arg(long, env = "BIDDER_PUBLIC_URL")]
    public_url: Option<String>,
    /// Seat ID returned in seatbid.seat.
    #[arg(long, env = "BIDDER_SEAT", default_value = "demo-seat")]
    seat: String,
    /// No-bid on requests whose tmax is below this (ms).
    #[arg(long, env = "BIDDER_MIN_TMAX_MS", default_value_t = 10)]
    min_tmax_ms: i32,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();
    let listener = tokio::net::TcpListener::bind((args.host.as_str(), args.port)).await?;
    let addr = listener.local_addr()?;
    let cfg = BidderConfig {
        public_url: args.public_url.unwrap_or_else(|| format!("http://{addr}")),
        seat: args.seat,
        min_tmax_ms: args.min_tmax_ms,
    };
    tracing::info!(%addr, public_url = %cfg.public_url, "demo-bidder listening");
    tokio::select! {
        r = demo_bidder::serve(listener, cfg) => r,
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("shutting down");
            Ok(())
        }
    }
}
