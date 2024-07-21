use pavex::blueprint::{
    router::{ANY, GET},
    Blueprint,
};
use pavex::f;

pub fn handler_1() -> pavex::response::Response {
    todo!()
}

pub fn handler_2() -> pavex::response::Response {
    todo!()
}

pub fn blueprint() -> Blueprint {
    let mut bp = Blueprint::new();
    bp.route(ANY, "/home", f!(crate::handler_1));
    bp.route(GET, "/home", f!(crate::handler_2));
    bp
}
