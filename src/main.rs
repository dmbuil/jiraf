//! jiraf: a lean, stateless follow-up dashboard over Jira (Data Center) Plans.
//!
//! Startup: load `.env` and `jiraf.toml` ([`config`]), check the token against Jira, resolve the
//! Extended Status field name, then serve the page and JSON API ([`api`]) on localhost, guarded by
//! [`local_only`]. All Jira traffic goes through [`jira`].

mod api;
mod config;
mod jira;

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::{Logger, Next, from_fn};
use actix_web::{App, Error, HttpServer, error, http::Method, web};
use anyhow::Context;

/// The server only ever listens on localhost, but a browser can still be tricked into talking to it:
/// reject foreign Host headers (DNS rebinding) and require a custom header on writes, which other
/// origins can't send without a CORS preflight we never grant.
async fn local_only(req: ServiceRequest, next: Next<impl MessageBody>) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let allowed = req.app_data::<web::Data<Vec<String>>>().cloned();
    let host = req.connection_info().host().to_string();
    if let Some(allowed) = allowed
        && !allowed.contains(&host)
    {
        return Err(error::ErrorForbidden("unexpected Host header"));
    }
    if req.method() != Method::GET {
        if req.headers().get("x-jiraf").and_then(|v| v.to_str().ok()) != Some("1") {
            return Err(error::ErrorForbidden("missing X-Jiraf header"));
        }
        if let Some(origin) = req.headers().get("origin").and_then(|v| v.to_str().ok())
            && origin != format!("http://{host}")
        {
            return Err(error::ErrorForbidden("cross-origin write"));
        }
    }
    next.call(req).await
}

/// Entry point: `jiraf`, reading `jiraf.toml` and `.env` from the current directory.
///
/// # Errors
/// Fails fast on a bad config, an invalid token, an unknown Extended Status field id, or a busy port.
#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info,actix_server=warn")).init();

    let dir = std::env::current_dir()?;
    config::ensure_dir(&dir)?;
    let cfg = config::Config::load(&dir)?;
    let jira = jira::Jira::new(&cfg)?;

    let me = jira.get("/rest/api/2/myself").await.context("checking JIRA_TOKEN against Jira")?;
    log::info!("connected to {} as {}", jira.base(), me["displayName"].as_str().unwrap_or("?"));
    let names = jira::field_names(&jira).await?;
    let ext_field_name = names
        .get(&cfg.fields.extended_status)
        .cloned()
        .with_context(|| format!("field {} (extended_status) not found in Jira", cfg.fields.extended_status))?;

    let bind = cfg.server.bind.clone().unwrap_or_else(|| "127.0.0.1:8080".into());
    let icon_dir = cfg.icon_dir.clone();
    let port = bind.rsplit(':').next().unwrap_or("8080").to_string();
    let hosts: Vec<String> = ["127.0.0.1", "localhost", "[::1]"].iter().map(|h| format!("{h}:{port}")).collect();
    let state = web::Data::new(api::AppState {
        cfg,
        jira,
        ext_field_name,
        icon_dir,
        cache: Default::default(),
        icons: Default::default(),
    });

    log::info!("jiraf on http://{bind}/");
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(web::Data::new(hosts.clone()))
            .wrap(from_fn(local_only))
            .wrap(Logger::new("%r %s %Dms"))
            .service(api::index)
            .service(api::app_js)
            .service(api::icon)
            .service(api::data)
            .service(api::transitions)
            .service(api::priorities)
            .service(api::assignable)
            .service(api::set_ext)
            .service(api::set_priority)
            .service(api::set_assignee)
            .service(api::set_target)
            .service(api::set_due)
            .service(api::transition)
    })
    .bind(&bind)
    .with_context(|| format!("binding {bind}"))?
    .run()
    .await?;
    Ok(())
}
