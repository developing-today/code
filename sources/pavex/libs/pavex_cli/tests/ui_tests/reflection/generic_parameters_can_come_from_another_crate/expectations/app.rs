//! Do NOT edit this code.
//! It was automatically generated by Pavex.
//! All manual edits will be lost next time the code is generated.
#[allow(unused_imports)]
use std as alloc;
struct ServerState {
    router: pavex::routing::Router<u32>,
    #[allow(dead_code)]
    application_state: ApplicationState,
}
pub struct ApplicationState {}
pub async fn build_application_state() -> crate::ApplicationState {
    crate::ApplicationState {}
}
pub async fn run(
    server_builder: pavex::hyper::server::Builder<
        pavex::hyper::server::conn::AddrIncoming,
    >,
    application_state: ApplicationState,
) -> Result<(), pavex::Error> {
    let server_state = std::sync::Arc::new(ServerState {
        router: build_router().map_err(pavex::Error::new)?,
        application_state,
    });
    let make_service = pavex::hyper::service::make_service_fn(move |_| {
        let server_state = server_state.clone();
        async move {
            Ok::<
                _,
                pavex::hyper::Error,
            >(
                pavex::hyper::service::service_fn(move |request| {
                    let server_state = server_state.clone();
                    async move {
                        let response = route_request(request, server_state).await;
                        let response = pavex::hyper::Response::from(response);
                        Ok::<_, pavex::hyper::Error>(response)
                    }
                }),
            )
        }
    });
    server_builder.serve(make_service).await.map_err(pavex::Error::new)
}
fn build_router() -> Result<pavex::routing::Router<u32>, pavex::routing::InsertError> {
    let mut router = pavex::routing::Router::new();
    router.insert("/home", 0u32)?;
    Ok(router)
}
async fn route_request(
    request: http::Request<pavex::hyper::body::Body>,
    server_state: std::sync::Arc<ServerState>,
) -> pavex::response::Response {
    #[allow(unused)]
    let (request_head, request_body) = request.into_parts();
    let request_head: pavex::request::RequestHead = request_head.into();
    let matched_route = match server_state.router.at(&request_head.uri.path()) {
        Ok(m) => m,
        Err(_) => {
            return pavex::response::Response::not_found().box_body();
        }
    };
    let route_id = matched_route.value;
    #[allow(unused)]
    let url_params: pavex::extract::route::RawRouteParams<'_, '_> = matched_route
        .params
        .into();
    match route_id {
        0u32 => {
            match &request_head.method {
                &pavex::http::Method::GET => route_handler_0().await,
                _ => {
                    let header_value = pavex::http::HeaderValue::from_static("GET");
                    pavex::response::Response::method_not_allowed()
                        .insert_header(pavex::http::header::ALLOW, header_value)
                        .box_body()
                }
            }
        }
        _ => pavex::response::Response::not_found().box_body(),
    }
}
pub async fn route_handler_0() -> pavex::response::Response {
    let v0 = app::handler();
    <pavex::response::Response<
        app::BodyType,
    > as pavex::response::IntoResponse>::into_response(v0)
}