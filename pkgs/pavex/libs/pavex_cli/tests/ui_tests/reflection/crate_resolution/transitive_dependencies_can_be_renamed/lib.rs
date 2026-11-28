use pavex::blueprint::{Blueprint, constructor::Lifecycle, router::GET};
use pavex::f;

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    dep::dep_blueprint(&mut bp);
    bp
}
