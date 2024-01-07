use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::anyhow;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use clap::Parser;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tracing::{info, info_span};
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .init();
    let args = Args::try_parse()?;

    info!("opening database {:?}...", args.db);
    let state: ServerState = Arc::new(Mutex::new(Persistence::open(args.db)?));
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/v1/links/:namespace", get(list_links))
        .route("/v1/links/:namespace", post(create_link))
        .route("/v1/links/:namespace/:short_form", get(get_link))
        .with_state(state);

    info!("listening at {}...", args.address);
    let listener = TcpListener::bind(args.address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

type ServerState = Arc<Mutex<Persistence>>;
struct Persistence {
    conn: rusqlite::Connection,
}
impl Persistence {
    #[tracing::instrument]
    fn open(path: PathBuf) -> anyhow::Result<Self> {
        let mut conn = rusqlite::Connection::open(path)?;
        schema::ensure_schema(&mut conn)?;
        Ok(Self { conn })
    }

    #[tracing::instrument(skip(self))]
    pub fn list_links(&mut self, namespace: String) -> anyhow::Result<Vec<Link>> {
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            self.conn.prepare(
                "SELECT short_form, long_form, created_at FROM links WHERE namespace = ?",
            )?
        };
        let links: Vec<Link> = {
            let _span = info_span!("query_map").entered();
            stmt.query_map([namespace], |row| {
                let link: Link = Link {
                    short_form: row.get(0)?,
                    long_form: row.get(1)?,
                    created_at: row.get(2)?,
                };
                Ok(link)
            })?
            .collect::<Result<Vec<_>, _>>()?
        };
        Ok(links)
    }

    #[tracing::instrument(skip(self))]
    pub fn get_link(
        &mut self,
        namespace: String,
        short_form: String,
    ) -> anyhow::Result<Option<Link>> {
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            self.conn.prepare(
                "SELECT long_form, created_at FROM links WHERE namespace = ? AND short_form = ?",
            )?
        };
        let links: Option<Link> = {
            let _span = info_span!("query_row").entered();
            stmt.query_row([namespace, short_form.clone()], |row| {
                let link: Link = Link {
                    short_form,
                    long_form: row.get(0)?,
                    created_at: row.get(1)?,
                };
                Ok(link)
            })
            .optional()?
        };
        Ok(links)
    }

    #[tracing::instrument(skip(self, link))]
    pub fn create_link(&mut self, namespace: String, link: Link) -> anyhow::Result<()> {
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            self.conn.prepare(
                "
                INSERT INTO links (namespace, short_form, long_form, created_at)
                VALUES (?, ?, ?, ?)
                ON CONFLICT (namespace, short_form)
                DO UPDATE SET
                    long_form = excluded.long_form,
                    created_at = excluded.created_at
            ",
            )?
        };
        info_span!("execute").in_scope(|| {
            stmt.execute((namespace, link.short_form, link.long_form, link.created_at))
        })?;
        Ok(())
    }
}

mod schema {
    const DDL_LINKS_TABLE: &str = "
        CREATE TABLE IF NOT EXISTS links (
            namespace TEXT NOT NULL,
            short_form TEXT NOT NULL,
            long_form TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (namespace, short_form)
        )
    ";
    pub(crate) fn ensure_schema(conn: &mut rusqlite::Connection) -> anyhow::Result<()> {
        conn.execute(DDL_LINKS_TABLE, [])?;
        Ok(())
    }
}

type AppResult<T> = Result<T, AppError>;
struct AppError(anyhow::Error);
impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "msg": self.0.to_string() })),
        )
            .into_response()
    }
}
impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self(value)
    }
}

#[derive(Serialize)]
struct Link {
    short_form: String,
    long_form: String,
    created_at: chrono::DateTime<Utc>,
}
#[derive(Serialize)]
struct ListLinksResponse {
    links: Vec<Link>,
}
async fn list_links(
    State(state): State<ServerState>,
    Path(namespace): Path<String>,
) -> AppResult<Json<ListLinksResponse>> {
    let links = state.lock().unwrap().list_links(namespace)?;
    Ok(Json(ListLinksResponse { links }))
}

#[derive(Deserialize)]
struct CreateLinkRequest {
    short_form: String,
    long_form: String,
}
#[derive(Serialize)]
struct CreateLinkResponse {}
async fn create_link(
    State(state): State<ServerState>,
    Path(namespace): Path<String>,
    Json(request): Json<CreateLinkRequest>,
) -> AppResult<Json<CreateLinkResponse>> {
    state.lock().unwrap().create_link(
        namespace,
        Link {
            short_form: request.short_form,
            long_form: request.long_form,
            created_at: chrono::Utc::now(),
        },
    )?;
    Ok(Json(CreateLinkResponse {}))
}

async fn get_link(
    State(state): State<ServerState>,
    Path((namespace, short_form)): Path<(String, String)>,
) -> AppResult<Json<Link>> {
    let Some(link) = state
        .lock()
        .unwrap()
        .get_link(namespace.clone(), short_form.clone())?
    else {
        return Err(anyhow!("no link {namespace}/{short_form}").into());
    };
    Ok(Json(link))
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:8080")]
    address: String,

    #[arg(long)]
    db: PathBuf,
}
