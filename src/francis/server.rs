use async_channel as mpsc;
use hyper::server::conn::AddrStream;
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use super::Command;

error_chain::error_chain! {
    errors {
        InvalidMethod(t: Method) {
            description("invalid http method")
            display("invalid http method: '{}'", t)
        }
        SendError {
            description("Send error occured")
            display("send error occured")
        }
    }
    foreign_links {
        Json(::serde_json::error::Error);
        Body(hyper::Error);
    }
}

async fn handle_post(context: mpsc::Sender<Command>, body: Body) -> Result<Response<Body>> {
    let body_bytes = hyper::body::to_bytes(body).await?;
    let body = serde_json::from_slice(&body_bytes)?;
    context.send(body).await.map_err(|_| ErrorKind::SendError)?;

    Ok(Response::new(Body::from("Aight")))
}

async fn handle(
    context: mpsc::Sender<Command>,
    req: Request<Body>,
    info: Arc<String>,
) -> std::result::Result<Response<Body>, Infallible> {
    let (parts, body) = req.into_parts();
    let resp = match parts.method {
        Method::POST => handle_post(context, body).await,
        Method::GET => Ok(Response::new(Body::from(info.to_string()))),
        _ => Err(ErrorKind::InvalidMethod(parts.method).into()),
    };

    match resp {
        Ok(x) => Ok(x),
        Err(e) => Ok(Response::new(Body::from(format!("Error {:?}", e)))),
    }
}

pub async fn start_server(port: u16, tx: mpsc::Sender<Command>, info: String) {
    // Construct our SocketAddr to listen on...
    let addr = SocketAddr::from(([127, 0, 0, 1], port));

    let info = Arc::new(info);

    // Shared is a MakeService that produces services by cloning an inner service...
    let make_service = make_service_fn(move |_conn: &AddrStream| {
        let tx = tx.clone();
        let info = info.clone();

        // Create a `Service` for responding to the request.
        let service = service_fn(move |req| handle(tx.clone(), req, info.clone()));

        // Return the service to hyper.
        async move { Ok::<_, Infallible>(service) }
    });

    // Then bind and serve...
    let server = Server::bind(&addr).serve(make_service);

    // And run forever...
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
}
