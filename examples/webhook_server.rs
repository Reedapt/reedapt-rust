//! Checks webhook deliveries with the signature header. Not a server: shows the calls to
//! make inside whichever web framework you use.

use reedapt::webhook::{self, EventData};

/// `body` must be the raw bytes as they arrived; `signature` is the `Reedapt-Signature` header.
fn handle(body: &[u8], signature: &str, secret: &str) -> u16 {
    let event = match webhook::construct_event(body, signature, secret) {
        Ok(event) => event,
        Err(_) => return 400,
    };
    match event.data {
        EventData::DubCompleted { dub } => println!("{} finished", dub.id),
        EventData::DubChecked { quality, .. } => println!("{} lines left to review", quality.to_review),
        // Ignore types you don't know; every endpoint gets every event.
        _ => {}
    }
    // An event can arrive more than once, so skip any `event.id` you've handled.
    200
}

fn main() {
    println!("{}", handle(b"{}", "t=1,v1=00", "whsec_example"));
}
