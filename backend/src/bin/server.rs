use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{anyhow, Context};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use clap::Parser;
use object_store::{aws::AmazonS3Builder, ObjectStore, PutPayload};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::{net::TcpListener, runtime::Handle, sync::Notify};
use tracing::{info, info_span, warn};
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .init();
    let args = Args::try_parse()?;
    if args.dotenv {
        dotenv::dotenv()?;
    }

    let cfg = Config {
        db_path: args.db_path,
        backup_staging_path: args.backup_staging_path,
        s3_region: args.s3_region,
        s3_bucket: args.s3_bucket,
        s3_path: args.s3_path,
    };
    let state: ServerState = Arc::new(Persistence::open(cfg).await?);
    let _backup_handle = tokio::task::spawn_blocking({
        let state = state.clone();
        move || {
            let h = Handle::current();
            let mut count = 0;
            loop {
                info!("awaiting dirty bit");
                h.block_on(state.dirty.notified());
                count += 1;
                info!(count, "triggering backup");
                let content = match state.stage_backup() {
                    Ok(content) => content,
                    Err(err) => {
                        warn!(?err, "failed to stage backup");
                        continue;
                    }
                };
                if let Err(err) = h.block_on(state.backup_to_s3(content)) {
                    warn!(?err, "failed to upload backup");
                    continue;
                }
            }
        }
    });
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/v1/links/:namespace", get(list_links))
        .route("/v1/links/:namespace", post(create_link))
        .route("/v1/links/:namespace/:short_form", get(get_link))
        .route("/v1/reverse_lookup/:namespace", post(reverse_lookup))
        .route("/v1/redirect/:namespace/:short_form", get(redirect_link))
        .with_state(state);

    info!("listening at {}...", args.address);
    let listener = TcpListener::bind(args.address).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

type ServerState = Arc<Persistence>;
struct Persistence {
    cfg: Config,
    conn: Mutex<rusqlite::Connection>,
    store: object_store::aws::AmazonS3,
    dirty: Notify,
}
#[derive(Debug)]
struct Config {
    db_path: std::path::PathBuf,
    backup_staging_path: std::path::PathBuf,
    s3_region: String,
    s3_bucket: String,
    s3_path: String,
}
impl Persistence {
    #[tracing::instrument]
    async fn open(cfg: Config) -> anyhow::Result<Self> {
        let _ = std::fs::remove_file(&cfg.db_path);
        let _ = std::fs::remove_file(&cfg.backup_staging_path);
        let store = AmazonS3Builder::from_env()
            .with_region(&cfg.s3_region)
            .with_bucket_name(&cfg.s3_bucket)
            .build()
            .context("init s3")?;
        {
            let get_response = store
                .get(&cfg.s3_path.as_str().into())
                .await
                .context("initial get db from s3")?;
            info!(?get_response, "found object");
            let payload = get_response.bytes().await?;
            info!(len = payload.len(), "downloaded object");
            std::fs::write(&cfg.db_path, payload)?;
        }
        let conn = Mutex::new(rusqlite::Connection::open(&cfg.db_path)?);
        Ok(Self {
            cfg,
            conn,
            store,
            dirty: Notify::new(),
        })
    }

    #[tracing::instrument(skip(self))]
    fn stage_backup(&self) -> anyhow::Result<Vec<u8>> {
        let conn = self.conn.lock().unwrap();
        let mut backup_conn = rusqlite::Connection::open(&self.cfg.backup_staging_path)?;
        let _span = info_span!("backup").entered();
        let b = rusqlite::backup::Backup::new(&conn, &mut backup_conn)?;
        b.run_to_completion(
            5,
            Duration::ZERO,
            Some(|p| {
                info!(?p, "backup tick");
            }),
        )?;
        let content = std::fs::read(&self.cfg.backup_staging_path)?;
        info!(size = content.len(), "read backup into memory");
        Ok(content)
    }

    #[tracing::instrument(skip(self, content))]
    async fn backup_to_s3(&self, content: Vec<u8>) -> anyhow::Result<()> {
        let put_response = self
            .store
            .put(&self.cfg.s3_path.as_str().into(), PutPayload::from(content))
            .await?;
        info!(?put_response, "finished uploading backup");
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    pub fn list_links(&self, namespace: String) -> anyhow::Result<Vec<Link>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            conn.prepare("SELECT short_form, long_form, created_at FROM links WHERE namespace = ?")?
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
    pub fn get_link(&self, namespace: String, short_form: String) -> anyhow::Result<Option<Link>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            conn.prepare(
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

    #[tracing::instrument(skip(self))]
    pub fn reverse_lookup(&self, namespace: String, long_form: String) -> anyhow::Result<Vec<Link>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            conn.prepare("SELECT short_form, long_form, created_at FROM links WHERE namespace = ? AND long_form = ?")?
        };
        let links: Vec<Link> = {
            let _span = info_span!("query_map").entered();
            stmt.query_map([namespace, long_form], |row| {
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

    #[tracing::instrument(skip(self, link))]
    pub fn create_link(&self, namespace: String, link: Link) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = {
            let _span = info_span!("prepare_statement").entered();
            conn.prepare(
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
        self.dirty.notify_one();
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
    let links = state.list_links(namespace)?;
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
    state.create_link(
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
    let Some(link) = state.get_link(namespace.clone(), short_form.clone())? else {
        return Err(anyhow!("no link {namespace}/{short_form}").into());
    };
    Ok(Json(link))
}

async fn redirect_link(
    State(state): State<ServerState>,
    Path((namespace, short_form)): Path<(String, String)>,
) -> AppResult<Redirect> {
    let Some(link) = state.get_link(namespace.clone(), short_form.clone())? else {
        return Err(anyhow!("no link {namespace}/{short_form}").into());
    };
    Ok(Redirect::temporary(&link.long_form))
}

#[derive(Deserialize)]
struct ReverseLookupRequest {
    long_form: String,
}
#[derive(Serialize)]
struct ReverseLookupResponse {
    links: Vec<Link>,
}
async fn reverse_lookup(
    State(state): State<ServerState>,
    Path(namespace): Path<String>,
    Json(ReverseLookupRequest { long_form }): Json<ReverseLookupRequest>,
) -> AppResult<Json<ReverseLookupResponse>> {
    let links = state.reverse_lookup(namespace.clone(), long_form.clone())?;
    Ok(Json(ReverseLookupResponse { links }))
}

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "[::]:8080")]
    address: String,

    #[arg(long)]
    s3_bucket: String,

    #[arg(long)]
    s3_region: String,

    #[arg(long)]
    s3_path: String,

    #[arg(long)]
    db_path: PathBuf,

    #[arg(long, help = "Where on disk to stage the backup db")]
    backup_staging_path: PathBuf,

    #[arg(long, help = "should we read .env?")]
    dotenv: bool,
}
