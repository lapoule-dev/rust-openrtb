use std::hint::black_box;

use buffa::{LazyMessageView, Message, MessageView};
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use openrtb_bench::{BIDSWITCH_JSON, bidswitch_request};
use openrtb_model::com::iabtechlab::openrtb::v2::{
    BidRequest, BidRequestView,
    __buffa::{lazy_view::BidRequestLazyView, oneof::bid_request::DistributionchannelOneof as Dc, view::oneof::bid_request::DistributionchannelOneof as DcView},
};

type IabReq = iab_specs_openrtb::v25::BidRequest<serde_json::Value>;

/// What a bidder typically reads before deciding to no-bid.
#[allow(dead_code)]
#[derive(Debug, Default)]
struct Peek<'a> {
    floor: f64,
    country: Option<&'a str>,
    devicetype: Option<i32>,
    bundle: Option<&'a str>,
}

fn bench(c: &mut Criterion) {
    let proto = bidswitch_request().encode_to_vec();
    // iab-specs only accepts `native.request` as a string, BidSwitch sends an object: stringify it.
    let mut v: serde_json::Value = serde_json::from_str(BIDSWITCH_JSON).unwrap();
    let req = &mut v["imp"][0]["native"]["request"];
    *req = serde_json::Value::String(req.to_string());
    let json_vec = serde_json::to_vec(&v).unwrap();
    let json = json_vec.as_slice();
    // sanity: iab-specs must parse the fixture
    let iab: IabReq = serde_json::from_slice(json).expect("iab-specs parse");

    let mut g = c.benchmark_group("decode");
    g.throughput(Throughput::Elements(1));
    g.bench_function("buffa/owned", |b| b.iter(|| BidRequest::decode_from_slice(black_box(&proto)).unwrap()));
    g.bench_function("buffa/view", |b| b.iter(|| BidRequestView::decode_view(black_box(&proto)).unwrap()));
    g.bench_function("buffa/lazy", |b| b.iter(|| BidRequestLazyView::decode_lazy(black_box(&proto)).unwrap()));
    g.bench_function("json/iab-specs", |b| b.iter(|| serde_json::from_slice::<IabReq>(black_box(json)).unwrap()));
    g.bench_function("json/serde_json::Value", |b| {
        b.iter(|| serde_json::from_slice::<serde_json::Value>(black_box(json)).unwrap())
    });
    g.finish();

    let mut g = c.benchmark_group("bidder_peek");
    g.bench_function("buffa/owned", |b| {
        b.iter(|| {
            let r = BidRequest::decode_from_slice(black_box(&proto)).unwrap();
            let d = &r.device;
            black_box((r.imp[0].bidfloor, d.geo.country.clone(), d.devicetype, match &r.distributionchannel_oneof { Some(Dc::App(a)) => a.bundle.clone(), _ => None }));
        })
    });
    g.bench_function("buffa/view", |b| {
        b.iter(|| {
            let r = BidRequestView::decode_view(black_box(&proto)).unwrap();
            let imp = r.imp.iter().next().unwrap();
            black_box(Peek {
                floor: imp.bidfloor.unwrap_or_default(),
                country: r.device.geo.country,
                devicetype: r.device.devicetype,
                bundle: match &r.distributionchannel_oneof { Some(DcView::App(a)) => a.bundle, _ => None },
            });
        })
    });
    g.bench_function("buffa/lazy", |b| {
        b.iter(|| {
            let r = BidRequestLazyView::decode_lazy(black_box(&proto)).unwrap();
            let imp = r.imp.iter().next().unwrap().unwrap();
            let device = r.device.get_or_default().unwrap();
            let geo = device.geo.get_or_default().unwrap();
            black_box(Peek {
                floor: imp.bidfloor.unwrap_or_default(),
                country: geo.country,
                devicetype: device.devicetype,
                bundle: match &r.distributionchannel_oneof { Some(DcView::App(a)) => a.bundle, _ => None },
            });
        })
    });
    g.bench_function("json/iab-specs", |b| {
        b.iter(|| {
            let r: IabReq = serde_json::from_slice(black_box(json)).unwrap();
            let d = r.device.as_ref().unwrap();
            black_box((r.imp[0].bidfloor, d.geo.as_ref().and_then(|g| g.country.clone()), d.devicetype));
        })
    });
    g.finish();

    let owned = bidswitch_request();
    let mut g = c.benchmark_group("encode");
    g.bench_function("buffa", |b| b.iter(|| black_box(&owned).encode_to_vec()));
    g.bench_function("json/iab-specs", |b| b.iter(|| serde_json::to_vec(black_box(&iab)).unwrap()));
    g.finish();

    eprintln!("sizes: proto={} B, json(pretty fixture)={} B, json(compact iab-specs)={} B",
        proto.len(), json.len(), serde_json::to_vec(&iab).unwrap().len());
}

criterion_group!(benches, bench);
criterion_main!(benches);
