use axum::{
    Router,
    extract::{Query, State, ws::Message},
    response::IntoResponse,
    routing::get,
};
use deployment::Deployment;
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    DeploymentImpl,
    middleware::signed_ws::{MaybeSignedWebSocket, SignedWsUpgrade},
};

#[derive(Debug, Deserialize)]
pub struct IssueStreamQuery {
    pub project_id: Uuid,
}

pub async fn stream_issues_ws(
    ws: SignedWsUpgrade,
    Query(query): Query<IssueStreamQuery>,
    State(deployment): State<DeploymentImpl>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| async move {
        if let Err(e) = handle_issues_ws(socket, deployment, query.project_id).await {
            tracing::warn!("issues WS closed: {}", e);
        }
    })
}

async fn handle_issues_ws(
    mut socket: MaybeSignedWebSocket,
    deployment: DeploymentImpl,
    project_id: Uuid,
) -> anyhow::Result<()> {
    use futures_util::{StreamExt, TryStreamExt};

    let mut stream = deployment
        .events()
        .stream_issues_raw(project_id)
        .await?
        .map_ok(|msg| msg.to_ws_message_unchecked());

    loop {
        tokio::select! {
            item = stream.next() => {
                match item {
                    Some(Ok(msg)) => {
                        if socket.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(e)) => {
                        tracing::error!("stream error: {}", e);
                        break;
                    }
                    None => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Ok(Some(Message::Close(_))) => break,
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }
    Ok(())
}

pub fn router() -> Router<DeploymentImpl> {
    Router::new().route("/issues/streams/ws", get(stream_issues_ws))
}
