use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode, Uri,
    body::{Bytes, Incoming},
    header::{HeaderValue, LOCATION, REFERER},
    service::service_fn,
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use std::{
    future::poll_fn, net::{Ipv4Addr, SocketAddr}, str::FromStr, sync::{Arc, LazyLock}, task::Poll,
};
use tokio::net::TcpListener;

use clap::Parser;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
};
use tokio_rustls::TlsAcceptor;

use local_ip_address::local_ip;

#[derive(Parser)]
struct RingCli {
    cert: Option<String>,

    #[arg(short, long)]
    port: Option<u16>,
}

// Based on https://github.com/rustls/hyper-rustls/blob/main/examples/server.rs

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = RingCli::parse();
    let port_env = std::env::var("PORT").ok().as_ref().and_then(|v| {
        u16::from_str(v).ok()
    });
    let port_int = port_env.unwrap_or(8080);
    let port = args.port.unwrap_or(port_int);
    let localhost_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);
    let network_addr = SocketAddr::new(local_ip()?, port);

    let cert_info = if let Some(c) = args.cert {
        let certs =
            CertificateDer::pem_file_iter(format!("{c}.pem"))?.collect::<Result<Vec<_>, _>>()?;
        let key = PrivateKeyDer::from_pem_file(format!("{c}.rsa"))?;
        Some((certs, key))
    } else {
        None
    };

    let incoming_localhost = TcpListener::bind(localhost_addr).await?;
    let incoming_network = TcpListener::bind(network_addr).await?;
    println!("Available on:");
    println!("{localhost_addr}");
    println!("{network_addr}");

    let tls_acceptor = if let Some((certs, key)) = cert_info {
        let mut server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        server_config.alpn_protocols =
            vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
        Some(TlsAcceptor::from(Arc::new(server_config)))
    } else {
        None
    };
    let service = service_fn(ring_service);

    loop {
        let (tcp_stream, _) = poll_fn(|ctx| {
            let network_poll = incoming_network.poll_accept(ctx);
            let local_poll = incoming_localhost.poll_accept(ctx);
            if let Poll::Ready(res) = local_poll {
                return Poll::Ready(res);
            }
            if let Poll::Ready(res) = network_poll {
                return Poll::Ready(res);
            }
            Poll::Pending
        }).await?;

        let tls_acceptor = tls_acceptor.clone();
        tokio::spawn(async move {
            if let Some(a) = tls_acceptor {
                let st = match a.accept(tcp_stream).await {
                    Ok(tls_stream) => tls_stream,
                    Err(err) => {
                        eprintln!("Handshake failed: {err:#}");
                        return;
                    }
                };
                if let Err(e) = Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(st), service)
                    .await
                {
                    eprintln!("Failed to serve connection: {e:#}");
                }
            } else {
                if let Err(e) = Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(tcp_stream), service)
                    .await
                {
                    eprintln!("Failed to serve HTTP connection: {e:#}");
                }
            }
        });
    }
}

fn read_from_public(path: &str) -> Result<Option<String>, String> {
    let to_read = std::path::Path::new(path);
    if let Some(f) = to_read.file_name() {
        let base = std::path::Path::new("./public");
        let pth = base.join(f);
        if pth.exists() {
            let file = std::fs::read_to_string(base.join(f)).map_err(|e| e.to_string())?;
            Ok(Some(file))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}

#[derive(Debug)]
struct Member {
    url: Uri,
    idx: usize,
}

static MEMBERS: LazyLock<Vec<Member>> =
    LazyLock::new(|| get_members().expect("Could not compile members list."));

fn get_members() -> Result<Vec<Member>, String> {
    let members_txt = read_from_public("members.txt")?.expect("members.txt does not exist.");
    let mut members = vec![];
    for (idx, member) in members_txt.lines().enumerate() {
        let uri =
            Uri::from_str(member).map_err(|e| format!("Could not parse member URI: {}", e))?;
        members.push(Member { idx, url: uri });
    }
    Ok(members)
}

fn redirect_to_member<T>(idx: usize, response: &mut Response<T>) -> Result<&Member, String> {
    *response.status_mut() = StatusCode::SEE_OTHER;
    let new_member = &MEMBERS[idx % MEMBERS.len()];
    let val: HeaderValue = new_member
        .url
        .to_string()
        .parse()
        .map_err(|e: hyper::header::InvalidHeaderValue| e.to_string())?;
    response.headers_mut().insert(LOCATION, val);
    Ok(new_member)
}

fn redirect_random<T>(response: &mut Response<T>) -> Result<&Member, String> {
    let idx = rand::random::<u64>() as usize;
    redirect_to_member(idx, response)
}

fn redirect_from_referer<T>(
    referer: Option<&HeaderValue>,
    add: isize,
    response: &mut Response<T>,
) -> Result<String, String> {
    let new_member = if let Some(r) = referer {
        let url_st = r.to_str().map_err(|e| e.to_string())?;
        let uri = Uri::from_str(url_st).map_err(|e| e.to_string())?;

        let member = MEMBERS
            .iter()
            .find(|member| member.url.authority() == uri.authority());

        if let Some(m) = member {
            redirect_to_member(m.idx.wrapping_add_signed(add), response)?
        } else {
            redirect_random(response)?
        }
    } else {
        redirect_random(response)?
    };
    Ok(format!("Redirecting to {}", new_member.url))
}

async fn ring_service(req: Request<Incoming>) -> Result<Response<Full<Bytes>>, String> {
    let mut response = Response::new(Full::default());
    // We only support GET methods.
    if req.method() != Method::GET {
        *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        return Ok(response);
    }
    let content = match req.uri().path() {
        "/" => read_from_public("index.html")?,
        "/left" | "/prev" | "/previous" => {
            let referer = req.headers().get(REFERER);
            Some(redirect_from_referer(referer, -1, &mut response)?)
        }
        "/right" | "/next" => {
            let referer = req.headers().get(REFERER);
            Some(redirect_from_referer(referer, 1, &mut response)?)
        }
        "/rand" | "/random" => Some(format!(
            "Redirecting to {}",
            redirect_random(&mut response)?.url
        )),
        "/members" => read_from_public("members.html")?,
        path => read_from_public(path)?,
    };
    if let Some(c) = content {
        *response.body_mut() = Full::from(c);
    } else {
        *response.status_mut() = StatusCode::NOT_FOUND;
        *response.body_mut() = Full::from("404: Not Found");
    }
    Ok(response)
}
