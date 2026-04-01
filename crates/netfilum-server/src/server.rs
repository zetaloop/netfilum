use crate::path::normalize_relative_path;
use filetime::{set_file_times, FileTime};
use netfilum::protocol::{
    BasicInfoUpdate, DirEntry, EntryKind, FileAttr, FileTimeValue, Request, Response, RpcError,
    RpcResult, VolumeInfoData,
};
use netfilum::rpc::{read_message, write_message};
use netfilum::{highlight, print_info};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_VOLUME_SIZE: u64 = 1 << 40;

pub fn run(
    root: String,
    addr: SocketAddr,
    volume_label: String,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let root = expand_root(&root);
    print_info(
        "serving",
        format_args!(
            "{} on {} as {}",
            highlight(root.display()),
            highlight(addr),
            highlight(&volume_label)
        ),
    );
    let server = RpcServer::new(root, volume_label)?;
    server.serve(addr)?;
    Ok(())
}

#[derive(Debug)]
struct RpcServer {
    root: PathBuf,
    root_real: PathBuf,
    volume_label: String,
}

impl RpcServer {
    fn new(root: PathBuf, volume_label: String) -> std::io::Result<Self> {
        fs::create_dir_all(&root)?;
        let root_real = root.canonicalize()?;
        Ok(Self {
            root,
            root_real,
            volume_label,
        })
    }

    fn serve(&self, addr: SocketAddr) -> std::io::Result<()> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(false)?;

        loop {
            let (stream, _) = listener.accept()?;
            if let Err(error) = self.handle_stream(stream) {
                eprintln!("netfilum server connection failed: {error}");
            }
        }
    }

    fn handle_stream(&self, mut stream: TcpStream) -> std::io::Result<()> {
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let request: Request = read_message(&mut stream)?;
        let response = self.dispatch(request);
        write_message(&mut stream, &response)
    }

    fn dispatch(&self, request: Request) -> RpcResult<Response> {
        match request {
            Request::Stat { path } | Request::Open { path } => {
                self.stat_path(&path).map(Response::Attr)
            }
            Request::Create {
                path,
                kind,
                file_attributes: _,
                allocation_size,
            } => self
                .create_entry(&path, kind, allocation_size)
                .map(Response::Attr),
            Request::ReadDir { path } => self.read_dir(&path).map(Response::DirEntries),
            Request::Read {
                path,
                offset,
                length,
            } => self.read_file(&path, offset, length).map(Response::Data),
            Request::Write {
                path,
                offset,
                data,
                write_to_eof,
            } => self.write_file(&path, offset, &data, write_to_eof),
            Request::CanDelete { path, kind } => {
                self.can_delete(&path, kind)?;
                Ok(Response::Empty)
            }
            Request::RemoveFile { path } => {
                self.remove_file(&path)?;
                Ok(Response::Empty)
            }
            Request::RemoveDir { path } => {
                self.remove_dir(&path)?;
                Ok(Response::Empty)
            }
            Request::Rename {
                path,
                new_path,
                replace_if_exists,
            } => {
                self.rename(&path, &new_path, replace_if_exists)?;
                Ok(Response::Empty)
            }
            Request::SetLen {
                path,
                size,
                set_allocation_size,
            } => self
                .set_len(&path, size, set_allocation_size)
                .map(Response::Attr),
            Request::Flush { path } => {
                self.flush(path.as_deref())?;
                Ok(Response::Empty)
            }
            Request::SetBasicInfo { path, update } => {
                self.set_basic_info(&path, update).map(Response::Attr)
            }
            Request::GetVolumeInfo => Ok(Response::VolumeInfo(VolumeInfoData {
                total_size: DEFAULT_VOLUME_SIZE,
                free_size: DEFAULT_VOLUME_SIZE / 2,
                label: self.volume_label.clone(),
            })),
        }
    }

    fn stat_path(&self, relative: &str) -> RpcResult<FileAttr> {
        let path = self.resolve_existing(relative)?;
        self.attr_for_path(&path)
    }

    fn create_entry(
        &self,
        relative: &str,
        kind: EntryKind,
        _allocation_size: u64,
    ) -> RpcResult<FileAttr> {
        let path = self.resolve_for_new_path(relative)?;
        match kind {
            EntryKind::File => {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(RpcError::from)?;
                file.flush().map_err(RpcError::from)?;
            }
            EntryKind::Directory => fs::create_dir(&path).map_err(RpcError::from)?,
        }
        self.attr_for_path(&path)
    }

    fn read_dir(&self, relative: &str) -> RpcResult<Vec<DirEntry>> {
        let path = self.resolve_existing(relative)?;
        let metadata = fs::metadata(&path).map_err(RpcError::from)?;
        if !metadata.is_dir() {
            return Err(RpcError::new(
                netfilum::protocol::ErrorCode::NotDirectory,
                "path is not a directory",
            ));
        }

        let mut entries = Vec::new();
        for entry in fs::read_dir(&path).map_err(RpcError::from)? {
            let entry = entry.map_err(RpcError::from)?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let attr = self.attr_for_path(&entry.path())?;
            entries.push(DirEntry { name, attr });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    fn read_file(&self, relative: &str, offset: u64, length: u32) -> RpcResult<Vec<u8>> {
        let path = self.resolve_existing(relative)?;
        let mut file = File::open(path).map_err(RpcError::from)?;
        file.seek(SeekFrom::Start(offset)).map_err(RpcError::from)?;
        let mut buffer = vec![0u8; length as usize];
        let read = file.read(&mut buffer).map_err(RpcError::from)?;
        buffer.truncate(read);
        Ok(buffer)
    }

    fn write_file(
        &self,
        relative: &str,
        offset: u64,
        data: &[u8],
        write_to_eof: bool,
    ) -> RpcResult<Response> {
        let path = self.resolve_existing(relative)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(RpcError::from)?;

        if write_to_eof {
            file.seek(SeekFrom::End(0)).map_err(RpcError::from)?;
        } else {
            file.seek(SeekFrom::Start(offset)).map_err(RpcError::from)?;
        }
        file.write_all(data).map_err(RpcError::from)?;
        file.flush().map_err(RpcError::from)?;

        Ok(Response::WriteResult {
            written: data.len() as u32,
            attr: self.attr_for_path(&path)?,
        })
    }

    fn can_delete(&self, relative: &str, kind: EntryKind) -> RpcResult<()> {
        let path = self.resolve_existing(relative)?;
        match kind {
            EntryKind::File => {
                let metadata = fs::metadata(&path).map_err(RpcError::from)?;
                if metadata.is_dir() {
                    return Err(RpcError::new(
                        netfilum::protocol::ErrorCode::IsDirectory,
                        "expected a file",
                    ));
                }
            }
            EntryKind::Directory => {
                let metadata = fs::metadata(&path).map_err(RpcError::from)?;
                if !metadata.is_dir() {
                    return Err(RpcError::new(
                        netfilum::protocol::ErrorCode::NotDirectory,
                        "expected a directory",
                    ));
                }
                let mut children = fs::read_dir(&path).map_err(RpcError::from)?;
                if children.next().is_some() {
                    return Err(RpcError::new(
                        netfilum::protocol::ErrorCode::DirectoryNotEmpty,
                        "directory is not empty",
                    ));
                }
            }
        }
        Ok(())
    }

    fn remove_file(&self, relative: &str) -> RpcResult<()> {
        let path = self.resolve_existing(relative)?;
        fs::remove_file(path).map_err(RpcError::from)
    }

    fn remove_dir(&self, relative: &str) -> RpcResult<()> {
        let path = self.resolve_existing(relative)?;
        fs::remove_dir(path).map_err(RpcError::from)
    }

    fn rename(&self, from: &str, to: &str, replace_if_exists: bool) -> RpcResult<()> {
        let from_path = self.resolve_existing(from)?;
        let to_path = self.resolve_for_new_path(to)?;
        if !replace_if_exists && to_path.exists() {
            return Err(RpcError::new(
                netfilum::protocol::ErrorCode::AlreadyExists,
                "target already exists",
            ));
        }
        if replace_if_exists && to_path.is_dir() {
            fs::remove_dir_all(&to_path).map_err(RpcError::from)?;
        } else if replace_if_exists && to_path.is_file() {
            fs::remove_file(&to_path).map_err(RpcError::from)?;
        }
        fs::rename(from_path, to_path).map_err(RpcError::from)
    }

    fn set_len(&self, relative: &str, size: u64, set_allocation_size: bool) -> RpcResult<FileAttr> {
        let path = self.resolve_existing(relative)?;
        if !set_allocation_size {
            let file = OpenOptions::new()
                .write(true)
                .open(&path)
                .map_err(RpcError::from)?;
            file.set_len(size).map_err(RpcError::from)?;
        }
        self.attr_for_path(&path)
    }

    fn flush(&self, relative: Option<&str>) -> RpcResult<()> {
        if let Some(relative) = relative {
            let path = self.resolve_existing(relative)?;
            if path.is_dir() {
                return Ok(());
            }
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
                .map_err(RpcError::from)?;
            file.sync_all().map_err(RpcError::from)?;
        }
        Ok(())
    }

    fn set_basic_info(&self, relative: &str, update: BasicInfoUpdate) -> RpcResult<FileAttr> {
        let path = self.resolve_existing(relative)?;

        if let Some(readonly) = update.readonly {
            set_readonly(&path, readonly).map_err(RpcError::from)?;
        }

        let current_atime = current_access_time(&path)?;
        let current_mtime = current_modified_time(&path)?;
        let next_atime = update
            .last_access_time
            .and_then(file_time_to_filetime)
            .unwrap_or(current_atime);
        let next_mtime = update
            .last_write_time
            .and_then(file_time_to_filetime)
            .unwrap_or(current_mtime);

        if update.last_access_time.is_some() || update.last_write_time.is_some() {
            set_file_times(&path, next_atime, next_mtime).map_err(RpcError::from)?;
        }

        self.attr_for_path(&path)
    }

    fn attr_for_path(&self, path: &Path) -> RpcResult<FileAttr> {
        let metadata = fs::metadata(path).map_err(RpcError::from)?;
        Ok(FileAttr {
            kind: if metadata.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            },
            size: metadata.len(),
            allocated_size: metadata.blocks() * 512,
            created: system_time_to_wire(metadata.created().ok()),
            accessed: system_time_to_wire(metadata.accessed().ok()),
            modified: system_time_to_wire(metadata.modified().ok()),
            changed: Some(FileTimeValue {
                secs: metadata.ctime(),
                nanos: metadata.ctime_nsec() as u32,
            }),
            readonly: metadata.permissions().mode() & 0o222 == 0,
            mode: Some(metadata.permissions().mode()),
        })
    }

    fn resolve_existing(&self, relative: &str) -> RpcResult<PathBuf> {
        let joined = self
            .root
            .join(normalize_relative_path(relative).map_err(|message| {
                RpcError::new(netfilum::protocol::ErrorCode::InvalidInput, message)
            })?);
        let resolved = joined.canonicalize().map_err(RpcError::from)?;
        self.ensure_within_root(&resolved)?;
        Ok(joined)
    }

    fn resolve_for_new_path(&self, relative: &str) -> RpcResult<PathBuf> {
        let joined = self
            .root
            .join(normalize_relative_path(relative).map_err(|message| {
                RpcError::new(netfilum::protocol::ErrorCode::InvalidInput, message)
            })?);
        let parent = joined.parent().unwrap_or(&self.root);
        let resolved_parent = parent.canonicalize().map_err(RpcError::from)?;
        self.ensure_within_root(&resolved_parent)?;
        Ok(joined)
    }

    fn ensure_within_root(&self, path: &Path) -> RpcResult<()> {
        if path.starts_with(&self.root_real) {
            Ok(())
        } else {
            Err(RpcError::new(
                netfilum::protocol::ErrorCode::PermissionDenied,
                "path escapes the exported root",
            ))
        }
    }
}

fn expand_root(raw: &str) -> PathBuf {
    if raw.contains("$USER") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(raw.replace("/home/$USER", &home))
    } else if raw == "~" {
        std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else if let Some(stripped) = raw.strip_prefix("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(stripped)
    } else {
        PathBuf::from(raw)
    }
}

fn system_time_to_wire(value: Option<SystemTime>) -> Option<FileTimeValue> {
    let time = value?;
    let duration = time.duration_since(UNIX_EPOCH).ok()?;
    Some(FileTimeValue {
        secs: duration.as_secs() as i64,
        nanos: duration.subsec_nanos(),
    })
}

fn file_time_to_filetime(value: FileTimeValue) -> Option<FileTime> {
    if value.secs < 0 {
        return None;
    }
    Some(FileTime::from_unix_time(value.secs, value.nanos))
}

fn current_access_time(path: &Path) -> RpcResult<FileTime> {
    let accessed = fs::metadata(path).map_err(RpcError::from)?.accessed().ok();
    Ok(accessed
        .and_then(|time| system_time_to_wire(Some(time)))
        .and_then(file_time_to_filetime)
        .unwrap_or_else(|| FileTime::from_system_time(SystemTime::now())))
}

fn current_modified_time(path: &Path) -> RpcResult<FileTime> {
    let modified = fs::metadata(path).map_err(RpcError::from)?.modified().ok();
    Ok(modified
        .and_then(|time| system_time_to_wire(Some(time)))
        .and_then(file_time_to_filetime)
        .unwrap_or_else(|| FileTime::from_system_time(SystemTime::now())))
}

fn set_readonly(path: &Path, readonly: bool) -> std::io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    let mut mode = permissions.mode();
    if readonly {
        mode &= !0o222;
    } else {
        mode |= 0o200;
    }
    permissions.set_mode(mode);
    fs::set_permissions(path, permissions)
}
