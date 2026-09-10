//! Output-target preflight for paired native archives.
//!
//! Rejects aliases before truncation and keeps params input distinct from outputs.
//! It does not provide atomic multi-file commits or protect against concurrent renames.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::Result;
use crate::cli::RunArgs;

#[cfg(windows)]
mod windows;

pub(crate) type Output = BufWriter<Box<dyn Write>>;
type FileKey = (u64, u64);

pub(crate) fn validate(args: &RunArgs) -> Result<()> {
    let metrics = args.metrics.as_deref().map(identity).transpose()?;
    let history = args.history.as_deref().map(identity).transpose()?;
    let params = args.params.as_deref().map(file_identity).transpose()?;
    if let (Some(a), Some(b)) = (&metrics, &history)
        && a.aliases(b)
    {
        return Err(conflict("--metrics and --history must use distinct outputs").into());
    }
    for output in [metrics, history].into_iter().flatten() {
        if params.as_ref().is_some_and(|params| output.aliases(params)) {
            return Err(conflict("output must not overwrite --params input").into());
        }
    }
    Ok(())
}

pub(crate) fn open(args: &RunArgs) -> Result<(Option<Output>, Option<Output>)> {
    // Open both without truncating, so failure to open the second target does
    // not destroy the first target's previous contents.
    let metrics = args
        .metrics
        .as_deref()
        .map(|path| open_target(path, "metrics"))
        .transpose()?;
    let history = args
        .history
        .as_deref()
        .map(|path| open_target(path, "history"))
        .transpose()?;
    let metrics_key = metrics.as_ref().map(Target::key).transpose()?.flatten();
    let history_key = history.as_ref().map(Target::key).transpose()?.flatten();
    if metrics_key.is_some() && metrics_key == history_key {
        // Two previously absent, differently cased names can become the same file
        // on a case-insensitive filesystem. Compare open handles before truncating.
        return Err(conflict("--metrics and --history must use distinct outputs").into());
    }
    Ok((
        metrics.map(Target::writer).transpose()?,
        history.map(Target::writer).transpose()?,
    ))
}

enum Target {
    Stdout,
    File(File),
}

impl Target {
    fn key(&self) -> Result<Option<FileKey>> {
        match self {
            Self::Stdout => stdout_key(),
            Self::File(file) => {
                #[cfg(windows)]
                {
                    windows::stream_key(file)
                }
                #[cfg(not(windows))]
                {
                    file_key(Path::new(""), &file.metadata()?).map(Some)
                }
            }
        }
    }

    fn writer(self) -> Result<Output> {
        let writer: Box<dyn Write> = match self {
            Self::Stdout => Box::new(io::stdout()),
            Self::File(file) => {
                // Pipes/devices are valid streams, but are not truncatable files.
                if file.metadata()?.is_file() {
                    file.set_len(0)?;
                }
                Box::new(file)
            }
        };
        Ok(BufWriter::new(writer))
    }
}

fn open_target(path: &Path, name: &str) -> Result<Target> {
    if path == Path::new("-") {
        return Ok(Target::Stdout);
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("could not open {name} {}: {error}", path.display()),
            )
        })?;
    Ok(Target::File(file))
}

enum Identity {
    Stdout(Option<FileKey>),
    File { path: PathBuf, key: Option<FileKey> },
}

impl Identity {
    fn aliases(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Stdout(_), Self::Stdout(_)) => true,
            (Self::File { path: a, key: ka }, Self::File { path: b, key: kb }) => {
                a == b || ka.is_some_and(|a| Some(a) == *kb)
            }
            (Self::Stdout(stdout), Self::File { key, .. })
            | (Self::File { key, .. }, Self::Stdout(stdout)) => stdout.is_some() && stdout == key,
        }
    }
}

fn identity(path: &Path) -> Result<Identity> {
    if path == Path::new("-") {
        Ok(Identity::Stdout(stdout_key()?))
    } else {
        file_identity(path)
    }
}

fn file_identity(path: &Path) -> Result<Identity> {
    let absolute = std::path::absolute(path)?;
    match fs::metadata(&absolute) {
        Ok(metadata) => Ok(Identity::File {
            path: absolute.canonicalize()?,
            key: Some(file_key(&absolute, &metadata)?),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A dangling symlink is not a new file; reject it rather than writing
            // through an unresolved alias.
            match fs::symlink_metadata(&absolute) {
                Ok(_) => {
                    return Err(
                        conflict(format!("cannot resolve output path {}", path.display())).into(),
                    );
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let parent = absolute
                .parent()
                .ok_or_else(|| conflict("output has no parent directory"))?;
            let name = absolute
                .file_name()
                .ok_or_else(|| conflict("output has no filename"))?;
            Ok(Identity::File {
                path: parent.canonicalize()?.join(name),
                key: None,
            })
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn file_key(_path: &Path, metadata: &Metadata) -> Result<FileKey> {
    use std::os::unix::fs::MetadataExt;
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(unix)]
fn stdout_key() -> Result<Option<FileKey>> {
    use std::os::fd::AsFd;
    let file = File::from(io::stdout().as_fd().try_clone_to_owned()?);
    file_key(Path::new("-"), &file.metadata()?).map(Some)
}

#[cfg(windows)]
fn file_key(path: &Path, _metadata: &Metadata) -> Result<FileKey> {
    use std::os::windows::fs::OpenOptionsExt;
    let file = OpenOptions::new().access_mode(0).open(path)?;
    windows::file_key(&file)
}

#[cfg(windows)]
fn stdout_key() -> Result<Option<FileKey>> {
    use std::os::windows::io::AsHandle;
    let file = File::from(io::stdout().as_handle().try_clone_to_owned()?);
    windows::stream_key(&file)
}

#[cfg(not(any(unix, windows)))]
fn file_key(_path: &Path, _metadata: &Metadata) -> Result<FileKey> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "output alias detection requires Unix or Windows",
    )
    .into())
}

#[cfg(not(any(unix, windows)))]
fn stdout_key() -> Result<Option<FileKey>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "output alias detection requires Unix or Windows",
    )
    .into())
}

fn conflict(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
