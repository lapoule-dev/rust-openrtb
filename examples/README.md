# Examples: a demo bidder and a demo SSP

Two small programs that speak OpenRTB 2.6 to each other through
`openrtb-model`, over JSON or protobuf:

| Crate | Role |
|---|---|
| [`demo-bidder`](demo-bidder) | DSP side: an axum server answering `POST /openrtb2/bid` from an in-memory campaign index, with win/billing notice endpoints and counters |
| [`demo-ssp`](demo-ssp) | Exchange / ad server side: replays randomized fixture requests to one or more bidders, runs the auction, fires the winners' `nurl`/`burl` and prints a report |

## Running

Terminal 1, the bidder (port 8080 by default):

```sh
cargo run --release -p demo-bidder
# options: --port / BIDDER_PORT, --host, --public-url (base of nurl/burl), --seat, --min-tmax-ms
# RUST_LOG=debug logs every bid, RUST_LOG=warn silences the win/bill log lines
```

Terminal 2, the SSP:

```sh
# 1000 auctions over JSON, first price, 32 in flight
cargo run --release -p demo-ssp

# protobuf on the wire, second price, 20k auctions, 64 in flight
cargo run --release -p demo-ssp -- -f protobuf -a second -n 20000 -c 64

# two bidders competing (start a second one with --port 8081 --seat other-seat)
cargo run --release -p demo-ssp -- -b http://127.0.0.1:8080/openrtb2/bid -b http://127.0.0.1:8081/openrtb2/bid

# a few auctions, one line each, reproducible traffic
cargo run --release -p demo-ssp -- -n 5 -v --seed 7
```

Then look at the bidder's counters:

```sh
curl -s localhost:8080/stats
# {"bids":12863,"bills":12863,"errors":0,"fast_no_bids":2585,"no_bids":7137,"requests":20000,"spend_usd":16.08,"wins":12863}
```

Other `demo-ssp` flags: `--tmax <ms>` overrides every request's `tmax`
(default: the fixture's, else 120 ms), `--no-notify` skips `nurl`/`burl`,
`--fixtures <dir>`, `--seed <n>`. `cargo run -p demo-ssp -- --help` lists them all.

A sample report (Apple M2, both binaries on one machine, release build):

```text
demo-ssp: 20000 auctions, 8 templates, 2 bidder(s), wire=Protobuf, auction=Second-price, concurrency=64

20000 auctions in 1.78 s: 11209 QPS

bidder #0 http://127.0.0.1:18080/openrtb2/bid
  bid rate 64.3% (12863 bids, 7137 no-bids, 0 timeouts, 0 errors, 0 invalid), 6467 imps won
  latency p50 1.82 ms  p90 2.96 ms  p99 4.67 ms  max 21.80 ms

bidder #1 http://127.0.0.1:18081/openrtb2/bid
  bid rate 64.3% (12863 bids, 7137 no-bids, 0 timeouts, 0 errors, 0 invalid), 6396 imps won
  latency p50 1.80 ms  p90 2.95 ms  p99 4.50 ms  max 21.59 ms

fill: 12863/20000 auctions (64.3%), 12863 imps won, 0 bids dropped (under floor / unknown imp)
clearing CPM (USD): min 0.2684  p50 1.0916  avg 1.1192  max 3.1521; spend 14.3966 USD
...
notices (nurl + burl): 25726 ok, 0 failed
```

These numbers measure the whole loop (SSP encode, HTTP/1.1 on loopback,
bidder decode and decision, SSP decode, auction, two notices per win), with
SSP and bidders sharing the same cores: they show the plumbing works, they
are not a bidder benchmark (see `crates/openrtb-bench` for codec numbers).

## Docker compose

One image holds the three binaries; the compose file runs two bidders, the
SSP against both, and (profile `load`) a goose load test of `bidder-a`.

```sh
cd examples
docker compose up --build bidder-a bidder-b ssp     # 20k auctions, report on the ssp logs
docker compose --profile load run --rm loadtest     # goose, 32 users, 5000 req/s cap, 30 s
docker compose down
```

Knobs: `SSP_FORMAT` (`json`|`protobuf`), `SSP_REQUESTS`, `LOAD_USERS`,
`LOAD_RUN_TIME`, `LOAD_RPS` (request cap: unthrottled, goose and the bidders
share the Docker VM's CPUs and can starve it; measure peak throughput natively). The bidders are published on ports 8080 and 8081
(`curl localhost:8080/stats`). The goose HTML report is written to
`examples/reports/goose-report.html`.

## Load testing with goose

`load-test` is a [goose](https://book.goose.rs) scenario: users post
randomized bid requests (built from `fixtures/`, new ids, floors ×0.3–×3,
random countries) to `/openrtb2/bid`, 3/4 over JSON and 1/4 over protobuf. A
request counts as failed unless it gets a 204, or a 200 whose body decodes as
a `BidResponse` with the request's id.

```sh
cargo run --release -p demo-bidder -- --port 8080 &
cargo run --release -p load-test -- --host http://127.0.0.1:8080 \
    --users 64 --hatch-rate 64 --run-time 30s --report-file report.html
```

All goose options apply (`--throttle-requests`, `--run-time`, `--report-file`, …).

## What the bidder does

`POST /openrtb2/bid`

- **Content negotiation**: `application/json` (or no `Content-Type`) is
  decoded with `OpenRtbJson`; `application/x-protobuf`, `application/protobuf`
  and `application/octet-stream` with buffa. The response uses the request's
  format. Anything else is `415`, an undecodable body `400`.
- **No bid** is `204 No Content` with an empty body. For protobuf, the request
  is first decoded as a **lazy view** (`BidRequestLazyView`): only `tmax`,
  `device.devicetype`, `device.geo.country` and each imp's
  `banner`/`native` presence and `bidfloor` are looked at. A sure no-bid
  (no banner/native imp, a floor above every campaign, an unreachable
  country/device, `tmax` below `--min-tmax-ms`) answers 204 without decoding
  the rest of the tree (`fast_no_bids` in `/stats`). Otherwise the lazy view
  becomes an owned `BidRequest` (`to_owned_message()`).
- **Campaigns** ([`campaigns.rs`](demo-bidder/src/campaigns.rs)): three native and three banner
  (300x250, 728x90, 300x600) campaigns, targeting by country (alpha-3, a few
  alpha-2 accepted), device type, format/size (`banner.format[]` and
  `banner.w/h`), excluded by `bcat` (a category blocks its subcategories:
  `IAB7` blocks `IAB7-39`) and `badv`, and each has a max CPM: an imp whose
  floor (converted from `bidfloorcur` with a static FX table) is at or above
  it is skipped. Private auctions (`pmp.private_auction = 1`) are skipped: the
  demo has no deals.
- **The bid** ([`bidder.rs`](demo-bidder/src/bidder.rs)): price shaded between the floor and the
  campaign max, rounded up so it never lands under the floor, in the first
  currency of `cur` the bidder knows (USD when `cur` is empty); `crid`, `cid`,
  `adomain`, `cat`, `mtype`, `w`/`h` for banners. `adm` is a **Native 1.2
  response** JSON string built from the request's assets (title truncated to
  `len`, images sized from `w`/`h` or `wmin`/`hmin`, data assets by type;
  a required asset it can't fill means no bid with that campaign), or banner
  HTML. The native request is read whether it came as an object or as the
  spec's JSON-encoded string.
- **Notices**: `nurl` = `GET /win`, `burl` = `GET /bill`, both carrying
  `${AUCTION_ID}`, `${AUCTION_BID_ID}`, `${AUCTION_IMP_ID}`,
  `${AUCTION_SEAT_ID}`, `${AUCTION_PRICE}`, `${AUCTION_CURRENCY}` and the
  campaign/creative ids. The handlers parse the substituted `price` and log it;
  an unsubstituted macro is a `400`.

`GET /stats`: requests, bids (responses with at least one bid), no-bids
(including the protobuf fast path), errors, wins, bills and spend (billed
CPMs / 1000, in USD).

## What the SSP does

1. Loads `fixtures/iab/*bidrequest*.json` and `fixtures/scala/*bidrequest*.json`.
2. Generates the traffic up front: for each auction a random template with a
   new `id` and `source.tid`, a random `device.geo.country` (10% none),
   floors scaled x0.3 to x3, 15% of requests priced in EUR (`cur` and
   `bidfloorcur`), `tmax` from the fixture, the flag or 120 ms.
3. Per auction, encodes the request once (JSON or protobuf) and posts it to
   every bidder concurrently, each call bounded by `tmax` (late = timeout).
4. Validates each response: `id` matches, `cur` allowed, each bid on a known
   imp and at or above its floor (bids that aren't are dropped and counted).
5. Runs a first-price or second-price auction per imp (second price: the
   winner pays max(second bid, floor) + 0.01, capped at its bid), comparing
   bids across currencies in USD.
6. Substitutes the auction macros in the winner's `nurl` and `burl` and calls
   them.
7. Prints QPS, per-bidder bid rate and latency p50/p90/p99/max, fill rate,
   clearing prices and spend, wins per creative and fill per fixture.

What to expect per fixture: the banner examples always fill (300x250 is open
to everyone), the mobile 728x90 one fills only in USA/GBR/CAN, the video one
never does (no video campaign; it's the protobuf fast no-bid path), the PMP
one never does (private auction), the native ones fill depending on country
and device.

## Mapping to scala-openrtb's examples

The scala-openrtb `examples` module
(`reference/scala-openrtb/examples/src/main/scala/com/powerspace/openrtb/examples/rtb/`)
has the same two roles:

| scala-openrtb | Here | Notes |
|---|---|---|
| `http4s/bidder/Bidder.scala`, `akkahttp/BidderApp.scala` (`RtbBidder`) | `demo-bidder/src/bidder.rs` | Scala bids on every native imp with a random price and a title-only native response. Here: campaign targeting, floors and currencies, every native asset type, banners. |
| `http4s/bidder/BidderHttpAppBuilder.scala`, akka-http `router` | `demo-bidder/src/server.rs` | Scala: JSON only, `POST /bid` (http4s) or `/bidOn` (akka), and a no-bid is `200` with an empty body. Here: JSON and protobuf on `/openrtb2/bid`, no-bid is `204`, plus `/win`, `/bill`, `/stats`. |
| `http4s/bidder/BidderApp.scala` | `demo-bidder/src/main.rs` | Server bootstrap. |
| `http4s/adserver/Adserver.scala` (`buildBidRequest`) | `demo-ssp/src/traffic.rs` | Scala builds one native request by hand; here fixture requests are randomized. |
| `http4s/adserver/AdserverHttpClientBuilder.scala`, `AdserverApp.scala` | `demo-ssp/src/main.rs`, `demo-ssp/src/auction.rs` | Scala sends one request and prints request + response as JSON. Here: many requests, several bidders, timeouts, the auction, macro substitution and notices, a report. |
| `http4s/common/ExampleSerdeModule.scala`, `example.proto` (`ImpExt`, `BidResponseExt`) | `OpenRtbJson` / `buffa::Message` | No custom serde module is needed: unknown JSON keys (`ext` included) are kept as is through both formats. Typed proto extensions aren't shown. |
