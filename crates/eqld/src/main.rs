use eqld::{
    config::Config,
    daemon::Daemon,
    diag, icons, install,
    lock::{self, Lock},
    skin, socials,
};
use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

const SUBCOMMANDS: &[&str] = &[
    "install-skin",
    "install-social",
    "install-tools",
    "upload-icons",
];

struct Invocation {
    path: PathBuf,
    force: bool,
    rest: Vec<String>,
}

/// `eqld [config.toml] [--force] [subcommand [args…]]` — the optional leading
/// config path is anything that is not a subcommand name or a global flag.
fn split_args(args: Vec<String>) -> Invocation {
    let mut path = None;
    let mut force = false;
    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.peek() {
        if arg == "--force" {
            force = true;
            iter.next();
            continue;
        }
        if SUBCOMMANDS.contains(&arg.as_str()) || path.is_some() {
            break;
        }
        path = iter.next().map(PathBuf::from);
    }
    Invocation {
        path: path.unwrap_or_else(|| PathBuf::from("eqld.toml")),
        force,
        rest: iter.collect(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Invocation { path, force, rest } = split_args(std::env::args().skip(1).collect());
    let config = Config::load(&path)?;

    let buffer = std::sync::Arc::new(diag::Buffer::new(diag::CAPACITY));
    let session = diag::session_id();
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer().with_ansi(config.log.colour()))
        .with(diag::Capture::new(buffer.clone()))
        .init();

    if let Some(subcommand) = rest.first() {
        return match subcommand.as_str() {
            "install-skin" => Ok(skin::run(&config, &rest[1..]).await?),
            "install-social" => Ok(socials::run(&config, &rest[1..])?),
            "install-tools" => Ok(install::run(&config, &rest[1..]).await?),
            "upload-icons" => Ok(icons::run(&config, &rest[1..]).await?),
            other => Err(format!("unknown subcommand {other:?}").into()),
        };
    }

    // One poll installs the handler before the lock exists: a ctrl-c racing
    // startup would otherwise kill the daemon and leave the lock behind.
    let mut interrupt = std::pin::pin!(tokio::signal::ctrl_c());
    std::future::poll_fn(|cx| {
        let _ = std::future::Future::poll(interrupt.as_mut(), cx);
        std::task::Poll::Ready(())
    })
    .await;

    let lock_path = lock::default_path(&config.state_path());
    let guard = match Lock::acquire(&lock_path, force) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let mut daemon = Daemon::new(config)?
        .watching(path.clone())
        .capturing(buffer, session);

    let harvest = daemon
        .config()
        .harvest_dir()
        .map_or_else(|| "disabled".into(), |dir| dir.display().to_string());
    tracing::info!(
        root = %daemon.config().game.root.display(),
        api = %daemon.config().api.url,
        poll_secs = daemon.config().poll_interval().as_secs(),
        state = %daemon.state_path().display(),
        state_exists = daemon.state_path().exists(),
        harvest = %harvest,
        overlays = ?daemon.overlays(),
        hidden = ?daemon.hidden_overlays(),
        headless = ?daemon.headless_overlays(),
        config = %path.display(),
        game_process = daemon.config().game_process().unwrap_or("undetectable"),
        skin = ?daemon.config().skin.wanted(),
        lock = %guard.path().display(),
        pid = guard.holder().pid,
        device = %daemon.config().log.device(),
        session = daemon.session().unwrap_or("off"),
        log_upload = daemon.config().log.upload,
        "eqld starting"
    );

    loop {
        daemon.tick().await;
        tokio::select! {
            _ = &mut interrupt => {
                tracing::info!("shutdown requested");
                break;
            }
            _ = tokio::time::sleep(daemon.delay()) => {}
        }
    }
    daemon.shutdown().await;
    guard.release();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(raw: &[&str]) -> Invocation {
        split_args(raw.iter().map(|arg| (*arg).to_string()).collect())
    }

    #[test]
    fn the_config_path_the_force_flag_and_the_subcommand_are_told_apart() {
        let bare = split(&[]);
        assert_eq!(bare.path, PathBuf::from("eqld.toml"));
        assert!(!bare.force);
        assert!(bare.rest.is_empty());

        let daemon = split(&["C:/eq/eqld.toml", "--force"]);
        assert_eq!(daemon.path, PathBuf::from("C:/eq/eqld.toml"));
        assert!(daemon.force);
        assert!(daemon.rest.is_empty());

        let leading = split(&["--force", "eqld.toml"]);
        assert!(leading.force);
        assert_eq!(leading.path, PathBuf::from("eqld.toml"));

        let sub = split(&["eqld.toml", "install-skin", "dorskui", "--skin", "v4"]);
        assert_eq!(sub.path, PathBuf::from("eqld.toml"));
        assert!(!sub.force);
        assert_eq!(sub.rest, ["install-skin", "dorskui", "--skin", "v4"]);

        let icons = split(&["upload-icons", "--force"]);
        assert_eq!(icons.path, PathBuf::from("eqld.toml"));
        assert!(
            !icons.force,
            "a flag after the subcommand belongs to the subcommand"
        );
        assert_eq!(icons.rest, ["upload-icons", "--force"]);
    }
}
