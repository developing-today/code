use pavex::blueprint::{Blueprint, router::GET};
use pavex::f;

pub struct Streamer;

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.route(GET, "/home", f!(crate::Streamer));
    bp
}
