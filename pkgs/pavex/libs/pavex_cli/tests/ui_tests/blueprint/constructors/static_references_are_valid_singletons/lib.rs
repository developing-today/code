use pavex::blueprint::{Blueprint, constructor::Lifecycle, router::GET};
use pavex::f;

pub fn static_str() -> &'static str {
    todo!()
}

pub fn handler(_x: &'static str) -> pavex::response::Response {
    todo!()
}

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.constructor(f!(crate::static_str), Lifecycle::Singleton);
    bp.route(GET, "/handler", f!(crate::handler));
    bp
}
