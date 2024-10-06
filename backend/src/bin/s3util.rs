use std::{io::Read, time::Duration};

use anyhow::bail;
use clap::{command, Parser, Subcommand};
use futures::StreamExt;
use object_store::{aws::AmazonS3Builder, ObjectStore, PutPayload};
use tracing::{info, info_span};
use tracing_subscriber::fmt::format::FmtSpan;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .init();

    let args = Args::try_parse()?;
    dotenv::dotenv()?;
    // from_env looks for:
    // - AWS_ACCESS_KEY_ID
    // - AWS_SECRET_ACCESS_KEY
    let store = AmazonS3Builder::from_env()
        .with_region("us-west-2")
        .with_bucket_name("flylinks-us-west-2")
        .build()?;

    match args.cmd {
        Command::List { prefix } => {
            let mut list_response = store.list(prefix.map(|p| p.into()).as_ref());
            let mut count = 0;
            while let Some(item) = list_response.next().await {
                println!("object: {:?}", item?);
                count += 1;
            }
            eprintln!("total objects={count}");
        }
        Command::Get { path, filename } => {
            let get_response = store.get(&path).await?;
            info!(?get_response, "found object");
            let payload = get_response.bytes().await?;
            info!(len = payload.len(), "downloaded object");
            std::fs::write(&filename, payload)?;
            info!(?filename, "wrote file");
        }
        Command::Put { path, content } => {
            let put_response = store.put(&path, PutPayload::from(content)).await?;
            eprintln!("put response={put_response:?}");
        }
        Command::Init { db } => {
            let mut conn = rusqlite::Connection::open(&db)?;
            schema::ensure_schema(&mut conn)?;
            info!(?db, "initialized database");
        }
        Command::Backup { db, path } => {
            let conn = rusqlite::Connection::open(&db)?;
            let mut tmp = tempfile::NamedTempFile::new()?;
            let mut backup_conn = rusqlite::Connection::open(&tmp)?;
            {
                let _span = info_span!("backup").entered();
                let b = rusqlite::backup::Backup::new(&conn, &mut backup_conn)?;
                b.run_to_completion(5, Duration::from_millis(100), None)?;
            }
            if let Err((_, err)) = backup_conn.close() {
                bail!("could not close backup: {err:?}");
            }
            info!(path = ?tmp.path(), "done with backup");
            let mut content = Vec::new();
            let size = tmp.read_to_end(&mut content)?;
            info!(size, "read backup into memory");
            let put_response = store.put(&path, PutPayload::from(content)).await?;
            info!(?put_response, "finished uploading backup");
        }
    };
    Ok(())
}

mod schema {
    const DDL_LINKS_TABLE: &str = "
        CREATE TABLE links (
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

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    List {
        #[arg(long)]
        prefix: Option<String>,
    },
    Get {
        #[arg(long)]
        path: object_store::path::Path,
        #[arg(long, help = "where to dump the contents to disk")]
        filename: std::path::PathBuf,
    },
    Put {
        #[arg(long)]
        path: object_store::path::Path,
        #[arg(long)]
        content: String,
    },
    Init {
        #[arg(long)]
        db: std::path::PathBuf,
    },
    Backup {
        #[arg(long)]
        db: std::path::PathBuf,
        #[arg(long, help = "where in s3 to dump the backup")]
        path: object_store::path::Path,
    },
}
