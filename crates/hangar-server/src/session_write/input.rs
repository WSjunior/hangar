//! `/input` e `/steer`. Provisório: admite e repassa; as Tasks seguintes trazem o corpo.
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::response::Response;

use super::{WriteRoute, through};
use crate::routes::AppState;

pub async fn input(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    through(&st, peer, req, WriteRoute::Input).await
}

pub async fn steer(State(st): State<Arc<AppState>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request) -> Response {
    through(&st, peer, req, WriteRoute::Steer).await
}
