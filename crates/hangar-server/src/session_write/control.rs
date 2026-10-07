//! `/interrupt`, `/select`, `/select/submit`, `/keys`, `/term-input` e `DELETE …/queue/{id}`.
//! Provisório: admite e repassa; as Tasks seguintes trazem o corpo.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::response::Response;

use super::{WriteRoute, through};
use crate::routes::AppState;

macro_rules! stub {
    ($name:ident, $route:expr) => {
        pub async fn $name(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
            through(&st, peer, req, $route).await
        }
    };
}

stub!(interrupt, WriteRoute::Interrupt);
stub!(select, WriteRoute::Select);
stub!(select_submit, WriteRoute::SelectSubmit);
stub!(keys, WriteRoute::Keys);
stub!(term_input, WriteRoute::TermInput);
stub!(queue_remove, WriteRoute::QueueRemove);
