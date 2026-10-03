use openrtb_model::v2::{BidRequest, bid_request};
fn main() {
    println!("BidRequest {}", size_of::<BidRequest>());
    println!("Imp {}", size_of::<bid_request::Imp>());
    println!("Device {}", size_of::<bid_request::Device>());
    println!("Geo {}", size_of::<bid_request::Geo>());
    println!("App {}", size_of::<bid_request::App>());
    println!("User {}", size_of::<bid_request::User>());
    println!("Source {}", size_of::<bid_request::Source>());
    println!("Regs {}", size_of::<bid_request::Regs>());
}
