use dep::{Logger, new_logger};
use pavex::blueprint::{Blueprint, constructor::Lifecycle, router::GET};
use pavex::f;

pub fn handler(logger: Logger) -> pavex::response::Response {
    todo!()
}

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.constructor(f!(new_logger), Lifecycle::Singleton);
    bp.route(GET, "/home", f!(crate::handler));
    bp
}
