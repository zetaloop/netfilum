use crate::path::windows_path_to_wsl;
use crate::protocol::{
    BasicInfoUpdate, DirEntry, EntryKind, FileAttr, FileTimeValue, Request, Response,
};
use crate::rpc::RpcClient;
use crate::{MountArgs, UpArgs, print_info, print_warn};
use std::ffi::c_void;
use std::io;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, HLOCAL, LocalFree, STATUS_CONNECTION_DISCONNECTED,
    STATUS_HOST_UNREACHABLE, STATUS_IO_TIMEOUT, STATUS_NETWORK_UNREACHABLE,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::PSECURITY_DESCRIPTOR;
use windows::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_READONLY, INVALID_FILE_ATTRIBUTES,
};
use windows::core::PCWSTR;
use winfsp::constants::FspCleanupFlags;
use winfsp::filesystem::{
    DirBuffer, DirInfo, DirMarker, FileInfo, FileSecurity, FileSystemContext,
    ModificationDescriptor, OpenFileInfo, VolumeInfo, WideNameInfo,
};
use winfsp::host::{FileSystemHost, VolumeParams};
use winfsp::{FspError, U16CStr, winfsp_init};
use winfsp_sys::{FILE_ACCESS_RIGHTS, FILE_FLAGS_AND_ATTRIBUTES};

const WINDOWS_EPOCH_OFFSET_SECS: i64 = 11_644_473_600;
const WINDOWS_TICKS_PER_SECOND: u64 = 10_000_000;
const SHUTDOWN_POLL: Duration = Duration::from_millis(200);
const CONNECTION_PROBE_INTERVAL: Duration = Duration::from_secs(1);
const SERVER_READY_TIMEOUT: Duration = Duration::from_secs(45);
const FILE_DIRECTORY_FILE_FLAG: u32 = 0x0000_0001;

pub fn run_mount(args: MountArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    print_info(format_args!(
        "netfilum: connecting to {} and preparing mount {}",
        args.addr, args.mount
    ));
    if args.password.is_empty() {
        print_warn(format_args!(
            "netfilum: warning: empty password configured, using plaintext transport"
        ));
    } else {
        print_info(format_args!(
            "netfilum: password authentication and encrypted transport enabled"
        ));
    }
    let _fsp = winfsp_init()?;
    let client = RpcClient::new(args.addr, args.password.clone());
    let descriptor = Arc::new(build_security_descriptor()?);
    let mount_state = Arc::new(MountState::default());

    let volume_label = match client.send(&Request::GetVolumeInfo) {
        Ok(Response::VolumeInfo(info)) if args.volume_label.is_empty() => info.label,
        Ok(Response::VolumeInfo(_)) => args.volume_label.clone(),
        Ok(_) => return Err("unexpected response while querying volume info".into()),
        Err(error) => {
            return Err(format!("failed to contact RPC server at {}: {error}", args.addr).into());
        }
    };

    let context = RpcFsContext {
        client,
        security_descriptor: descriptor,
        volume_label: volume_label.clone(),
        mount_state: Arc::clone(&mount_state),
    };
    let watcher =
        spawn_connection_monitor(args.addr, context.client.clone(), Arc::clone(&mount_state));

    let mut params = VolumeParams::new();
    params
        .filesystem_name("netfilum")
        .case_sensitive_search(true)
        .case_preserved_names(true)
        .unicode_on_disk(true)
        .flush_and_purge_on_cleanup(true)
        .post_cleanup_when_modified_only(false)
        .post_disposition_only_when_necessary(true)
        .supports_posix_unlink_rename(true)
        .sector_size(4096)
        .sectors_per_allocation_unit(1)
        .max_component_length(255)
        .file_info_timeout(1_000)
        .dir_info_timeout(1_000)
        .volume_info_timeout(1_000)
        .security_timeout(1_000);

    let mut host = FileSystemHost::new(params, context)?;
    host.start()?;
    host.mount(args.mount.as_str())?;
    print_info(format_args!(
        "netfilum: mounted {} on {}. Press Ctrl+C to unmount.",
        volume_label, args.mount
    ));

    let shutdown_reason = wait_for_shutdown(Arc::clone(&mount_state))?;
    if let ShutdownReason::ServerDisconnected(message) = shutdown_reason {
        print_warn(format_args!("{message}"));
    }

    mount_state.stop.store(true, Ordering::SeqCst);
    print_info(format_args!("netfilum: unmounting {}", args.mount));
    host.unmount();
    host.stop();
    let _ = watcher.join();
    Ok(())
}

pub fn run_up(args: UpArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    print_info(format_args!(
        "netfilum: starting netfilumd in WSL distro {} for {}",
        args.distro, args.addr
    ));
    let mut child = spawn_wsl_server(&args)?;
    let mount_args = MountArgs {
        mount: args.mount.clone(),
        addr: args.addr,
        volume_label: args.volume_label.clone(),
        password: args.password.clone(),
    };

    let result = (|| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        print_info(format_args!(
            "netfilum: waiting for RPC server at {}",
            args.addr
        ));
        wait_for_server(args.addr, &args.password, &mut child)?;
        print_info(format_args!("netfilum: RPC server is ready"));
        run_mount(mount_args)
    })();

    print_info(format_args!("netfilum: stopping WSL helper process"));
    stop_wsl_server(&mut child);
    result
}

fn spawn_wsl_server(args: &UpArgs) -> Result<Child, Box<dyn std::error::Error + Send + Sync>> {
    const DEFAULT_WSL_ROOT: &str = "/home/$USER/netfilum-root";

    let daemon = sibling_daemon_path()?;
    let daemon_wsl = windows_path_to_wsl(daemon.to_string_lossy().as_ref())
        .map_err(|error| format!("failed to map daemon path into WSL: {error}"))?;
    let root = if args.root == DEFAULT_WSL_ROOT {
        "\"$HOME/netfilum-root\"".to_string()
    } else {
        shell_quote(&args.root)
    };
    let command = format!(
        "set -e; exec {} --root {} --addr {} --volume-label {}",
        shell_quote(&daemon_wsl),
        root,
        shell_quote(&args.addr.to_string()),
        shell_quote(&args.volume_label)
    );
    let command = if args.password.is_empty() {
        command
    } else {
        format!("{command} --password {}", shell_quote(&args.password))
    };

    Command::new("wsl.exe")
        .args(["-d", args.distro.as_str(), "sh", "-lc", command.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to start WSL server in distro {}: {error}",
                args.distro
            )
            .into()
        })
}

fn sibling_daemon_path() -> Result<std::path::PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe.parent().ok_or_else(|| {
        format!(
            "failed to determine executable directory for {}",
            current_exe.display()
        )
    })?;
    let daemon = exe_dir.join("netfilumd");
    if daemon.is_file() {
        Ok(daemon)
    } else {
        Err(format!(
            "`netfilum up` requires a sibling Linux daemon binary at {}",
            daemon.display()
        )
        .into())
    }
}

fn wait_for_server(
    addr: std::net::SocketAddr,
    password: &str,
    child: &mut Child,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = RpcClient::new(addr, password.to_string());
    let deadline = Instant::now() + SERVER_READY_TIMEOUT;

    loop {
        let error = match client.send(&Request::GetVolumeInfo) {
            Ok(Response::VolumeInfo(_)) => return Ok(()),
            Ok(_) => return Err("unexpected response while waiting for RPC server".into()),
            Err(error) => error,
        };

        if let Some(status) = child.try_wait()? {
            return Err(format!("WSL server exited before becoming ready: {status}").into());
        }

        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for RPC server at {addr}: {error}").into());
        }

        thread::sleep(SHUTDOWN_POLL);
    }
}

fn stop_wsl_server(child: &mut Child) {
    match child.try_wait() {
        Ok(Some(_)) | Err(_) => return,
        Ok(None) => {}
    }

    let _ = child.kill();
    let _ = child.wait();
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[derive(Debug)]
struct RpcFsContext {
    client: RpcClient,
    security_descriptor: Arc<Vec<u8>>,
    volume_label: String,
    mount_state: Arc<MountState>,
}

#[derive(Debug)]
struct RpcFileContext {
    path: RwLock<String>,
    kind: EntryKind,
    dir_buffer: DirBuffer,
    delete_pending: AtomicBool,
}

impl RpcFileContext {
    fn new(path: String, kind: EntryKind) -> Self {
        Self {
            path: RwLock::new(path),
            kind,
            dir_buffer: DirBuffer::new(),
            delete_pending: AtomicBool::new(false),
        }
    }

    fn path(&self) -> String {
        self.path.read().expect("path lock poisoned").clone()
    }

    fn set_path(&self, path: String) {
        *self.path.write().expect("path lock poisoned") = path;
    }
}

impl FileSystemContext for RpcFsContext {
    type FileContext = RpcFileContext;

    fn get_security_by_name(
        &self,
        file_name: &U16CStr,
        security_descriptor: Option<&mut [c_void]>,
        resolve_reparse_points: impl FnOnce(&U16CStr) -> Option<FileSecurity>,
    ) -> winfsp::Result<FileSecurity> {
        if let Some(reparse) = resolve_reparse_points(file_name) {
            return Ok(reparse);
        }

        let path = nt_path_to_relative(file_name);
        let attr = self.fetch_attr(&path)?;
        copy_security_descriptor(&self.security_descriptor, security_descriptor);

        Ok(FileSecurity {
            reparse: false,
            sz_security_descriptor: self.security_descriptor.len() as u64,
            attributes: file_attributes_from_attr(&attr),
        })
    }

    fn open(
        &self,
        file_name: &U16CStr,
        _create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let path = nt_path_to_relative(file_name);
        let attr = self.fetch_attr(&path)?;
        fill_open_file_info(file_info, &attr);
        Ok(RpcFileContext::new(path, attr.kind))
    }

    fn close(&self, _context: Self::FileContext) {}

    fn create(
        &self,
        file_name: &U16CStr,
        create_options: u32,
        _granted_access: FILE_ACCESS_RIGHTS,
        file_attributes: FILE_FLAGS_AND_ATTRIBUTES,
        _security_descriptor: Option<&[c_void]>,
        allocation_size: u64,
        _extra_buffer: Option<&[u8]>,
        _extra_buffer_is_reparse_point: bool,
        file_info: &mut OpenFileInfo,
    ) -> winfsp::Result<Self::FileContext> {
        let path = nt_path_to_relative(file_name);
        let kind = if create_options & FILE_DIRECTORY_FILE_FLAG != 0 {
            EntryKind::Directory
        } else {
            EntryKind::File
        };

        let response = self.send_request(Request::Create {
            path: path.clone(),
            kind,
            file_attributes,
            allocation_size,
        })?;

        let Response::Attr(attr) = response else {
            return Err(FspError::from(io::Error::other(
                "unexpected create response",
            )));
        };

        fill_open_file_info(file_info, &attr);
        Ok(RpcFileContext::new(path, attr.kind))
    }

    fn cleanup(&self, context: &Self::FileContext, _file_name: Option<&U16CStr>, flags: u32) {
        let should_delete = context.delete_pending.load(Ordering::SeqCst)
            || FspCleanupFlags::FspCleanupDelete.is_flagged(flags);
        if !should_delete {
            return;
        }

        let request = match context.kind {
            EntryKind::File => Request::RemoveFile {
                path: context.path(),
            },
            EntryKind::Directory => Request::RemoveDir {
                path: context.path(),
            },
        };
        let _ = self.send_request(request);
    }

    fn flush(
        &self,
        context: Option<&Self::FileContext>,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        if let Some(context) = context {
            self.send_request(Request::Flush {
                path: Some(context.path()),
            })?;
            let attr = self.fetch_attr(&context.path())?;
            fill_file_info(file_info, &attr);
        } else {
            self.send_request(Request::Flush { path: None })?;
        }
        Ok(())
    }

    fn get_file_info(
        &self,
        context: &Self::FileContext,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let attr = self.fetch_attr(&context.path())?;
        fill_file_info(file_info, &attr);
        Ok(())
    }

    fn get_security(
        &self,
        _context: &Self::FileContext,
        security_descriptor: Option<&mut [c_void]>,
    ) -> winfsp::Result<u64> {
        copy_security_descriptor(&self.security_descriptor, security_descriptor);
        Ok(self.security_descriptor.len() as u64)
    }

    fn set_security(
        &self,
        _context: &Self::FileContext,
        _security_information: u32,
        _modification_descriptor: ModificationDescriptor,
    ) -> winfsp::Result<()> {
        Err(ERROR_ACCESS_DENIED.into())
    }

    fn read_directory(
        &self,
        context: &Self::FileContext,
        _pattern: Option<&U16CStr>,
        marker: DirMarker,
        buffer: &mut [u8],
    ) -> winfsp::Result<u32> {
        let path = context.path();
        let attr = self.fetch_attr(&path)?;
        let entries = self.fetch_dir_entries(&path)?;

        if let Ok(dir_buffer) = context
            .dir_buffer
            .acquire(marker.is_none(), Some((entries.len() + 2) as u32))
            && marker.is_none()
        {
            write_special_dir_entry(&dir_buffer, ".", &attr)?;
            write_special_dir_entry(&dir_buffer, "..", &attr)?;
            for entry in entries {
                write_dir_entry(&dir_buffer, &entry)?;
            }
        }

        Ok(context.dir_buffer.read(marker, buffer))
    }

    fn rename(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        new_file_name: &U16CStr,
        replace_if_exists: bool,
    ) -> winfsp::Result<()> {
        let old_path = context.path();
        let new_path = nt_path_to_relative(new_file_name);
        self.send_request(Request::Rename {
            path: old_path,
            new_path: new_path.clone(),
            replace_if_exists,
        })?;
        context.set_path(new_path);
        Ok(())
    }

    fn set_basic_info(
        &self,
        context: &Self::FileContext,
        file_attributes: u32,
        creation_time: u64,
        last_access_time: u64,
        last_write_time: u64,
        change_time: u64,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let response = self.send_request(Request::SetBasicInfo {
            path: context.path(),
            update: BasicInfoUpdate {
                readonly: readonly_update_from_attributes(file_attributes),
                creation_time: wire_time_from_windows(creation_time),
                last_access_time: wire_time_from_windows(last_access_time),
                last_write_time: wire_time_from_windows(last_write_time),
                change_time: wire_time_from_windows(change_time),
            },
        })?;

        let Response::Attr(attr) = response else {
            return Err(FspError::from(io::Error::other(
                "unexpected set_basic_info response",
            )));
        };

        fill_file_info(file_info, &attr);
        Ok(())
    }

    fn set_delete(
        &self,
        context: &Self::FileContext,
        _file_name: &U16CStr,
        delete_file: bool,
    ) -> winfsp::Result<()> {
        if delete_file {
            self.send_request(Request::CanDelete {
                path: context.path(),
                kind: context.kind,
            })?;
        }

        context.delete_pending.store(delete_file, Ordering::SeqCst);
        Ok(())
    }

    fn set_file_size(
        &self,
        context: &Self::FileContext,
        new_size: u64,
        set_allocation_size: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<()> {
        let response = self.send_request(Request::SetLen {
            path: context.path(),
            size: new_size,
            set_allocation_size,
        })?;

        let Response::Attr(attr) = response else {
            return Err(FspError::from(io::Error::other(
                "unexpected set_file_size response",
            )));
        };

        fill_file_info(file_info, &attr);
        Ok(())
    }

    fn read(
        &self,
        context: &Self::FileContext,
        buffer: &mut [u8],
        offset: u64,
    ) -> winfsp::Result<u32> {
        let response = self.send_request(Request::Read {
            path: context.path(),
            offset,
            length: buffer.len() as u32,
        })?;

        let Response::Data(data) = response else {
            return Err(FspError::from(io::Error::other("unexpected read response")));
        };

        let read = data.len();
        buffer[..read].copy_from_slice(&data);
        Ok(read as u32)
    }

    fn write(
        &self,
        context: &Self::FileContext,
        buffer: &[u8],
        offset: u64,
        write_to_eof: bool,
        constrained_io: bool,
        file_info: &mut FileInfo,
    ) -> winfsp::Result<u32> {
        let mut payload = buffer.to_vec();
        if constrained_io {
            let attr = self.fetch_attr(&context.path())?;
            if offset >= attr.size {
                return Ok(0);
            }

            let max_len = (attr.size - offset) as usize;
            if payload.len() > max_len {
                payload.truncate(max_len);
            }
        }

        let response = self.send_request(Request::Write {
            path: context.path(),
            offset,
            data: payload,
            write_to_eof,
        })?;

        let Response::WriteResult { written, attr } = response else {
            return Err(FspError::from(io::Error::other(
                "unexpected write response",
            )));
        };

        fill_file_info(file_info, &attr);
        Ok(written)
    }

    fn get_volume_info(&self, out_volume_info: &mut VolumeInfo) -> winfsp::Result<()> {
        let response = self.send_request(Request::GetVolumeInfo)?;

        let Response::VolumeInfo(info) = response else {
            return Err(FspError::from(io::Error::other(
                "unexpected volume response",
            )));
        };

        out_volume_info.total_size = info.total_size;
        out_volume_info.free_size = info.free_size;
        out_volume_info.set_volume_label(self.volume_label.as_str());
        Ok(())
    }
}

impl RpcFsContext {
    fn send_request(&self, request: Request) -> winfsp::Result<Response> {
        self.client
            .send(&request)
            .map_err(|error| map_rpc_error(error, &self.mount_state))
    }

    fn fetch_attr(&self, path: &str) -> winfsp::Result<FileAttr> {
        match self.send_request(Request::Stat {
            path: path.to_string(),
        })? {
            Response::Attr(attr) => Ok(attr),
            _ => Err(FspError::from(io::Error::other("unexpected attr response"))),
        }
    }

    fn fetch_dir_entries(&self, path: &str) -> winfsp::Result<Vec<DirEntry>> {
        match self.send_request(Request::ReadDir {
            path: path.to_string(),
        })? {
            Response::DirEntries(entries) => Ok(entries),
            _ => Err(FspError::from(io::Error::other(
                "unexpected directory response",
            ))),
        }
    }
}

#[derive(Debug, Default)]
struct MountState {
    stop: AtomicBool,
    disconnect_message: Mutex<Option<String>>,
}

#[derive(Debug)]
enum ShutdownReason {
    Interrupted,
    ServerDisconnected(String),
}

impl MountState {
    fn report_disconnect(&self, message: String) {
        let mut slot = self
            .disconnect_message
            .lock()
            .expect("disconnect message lock poisoned");
        if slot.is_none() {
            *slot = Some(message);
        }
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn spawn_connection_monitor(
    addr: std::net::SocketAddr,
    client: RpcClient,
    mount_state: Arc<MountState>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        loop {
            thread::sleep(CONNECTION_PROBE_INTERVAL);
            if mount_state.stop.load(Ordering::SeqCst) {
                break;
            }

            match client.send(&Request::GetVolumeInfo) {
                Ok(Response::VolumeInfo(_)) => {}
                Ok(_) => mount_state.report_disconnect(format!(
                    "netfilum: RPC server at {addr} returned an unexpected response"
                )),
                Err(error) if is_disconnect_error(&error) => mount_state.report_disconnect(
                    format!("netfilum: lost connection to RPC server at {addr}: {error}"),
                ),
                Err(_) => {}
            }

            if mount_state.stop.load(Ordering::SeqCst) {
                break;
            }
        }
    })
}

fn wait_for_shutdown(
    mount_state: Arc<MountState>,
) -> Result<ShutdownReason, Box<dyn std::error::Error + Send + Sync>> {
    let signal = Arc::clone(&mount_state);
    ctrlc::set_handler(move || {
        signal.stop.store(true, Ordering::SeqCst);
    })?;

    while !mount_state.stop.load(Ordering::SeqCst) {
        thread::sleep(SHUTDOWN_POLL);
    }

    let message = mount_state
        .disconnect_message
        .lock()
        .expect("disconnect message lock poisoned")
        .clone();
    Ok(match message {
        Some(message) => ShutdownReason::ServerDisconnected(message),
        None => ShutdownReason::Interrupted,
    })
}

fn map_rpc_error(error: io::Error, mount_state: &MountState) -> FspError {
    if let Some(status) = disconnect_status(&error) {
        mount_state.report_disconnect(format!("netfilum: lost connection to RPC server: {error}"));
        return FspError::NTSTATUS(status.0);
    }

    FspError::from(error)
}

fn disconnect_status(error: &io::Error) -> Option<windows::Win32::Foundation::NTSTATUS> {
    match error.kind() {
        io::ErrorKind::ConnectionAborted
        | io::ErrorKind::ConnectionRefused
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::NotConnected
        | io::ErrorKind::UnexpectedEof => Some(STATUS_CONNECTION_DISCONNECTED),
        io::ErrorKind::HostUnreachable => Some(STATUS_HOST_UNREACHABLE),
        io::ErrorKind::NetworkUnreachable => Some(STATUS_NETWORK_UNREACHABLE),
        io::ErrorKind::TimedOut => Some(STATUS_IO_TIMEOUT),
        _ => None,
    }
}

fn is_disconnect_error(error: &io::Error) -> bool {
    disconnect_status(error).is_some()
}

fn write_special_dir_entry(
    dir_buffer: &winfsp::filesystem::DirBufferLock<'_>,
    name: &str,
    attr: &FileAttr,
) -> winfsp::Result<()> {
    let mut dir_info = DirInfo::<255>::new();
    dir_info.set_name(name)?;
    fill_file_info(dir_info.file_info_mut(), attr);
    dir_buffer.write(&mut dir_info)
}

fn write_dir_entry(
    dir_buffer: &winfsp::filesystem::DirBufferLock<'_>,
    entry: &DirEntry,
) -> winfsp::Result<()> {
    let mut dir_info = DirInfo::<255>::new();
    dir_info.set_name(entry.name.as_str())?;
    fill_file_info(dir_info.file_info_mut(), &entry.attr);
    dir_buffer.write(&mut dir_info)
}

fn fill_open_file_info(file_info: &mut OpenFileInfo, attr: &FileAttr) {
    fill_file_info(file_info.as_mut(), attr);
}

fn fill_file_info(file_info: &mut FileInfo, attr: &FileAttr) {
    file_info.file_attributes = file_attributes_from_attr(attr);
    file_info.reparse_tag = 0;
    file_info.allocation_size = attr.allocated_size.max(attr.size);
    file_info.file_size = attr.size;
    file_info.creation_time = windows_time_from_wire(attr.created);
    file_info.last_access_time = windows_time_from_wire(attr.accessed);
    file_info.last_write_time = windows_time_from_wire(attr.modified);
    file_info.change_time = windows_time_from_wire(attr.changed.or(attr.modified));
    file_info.index_number = 0;
    file_info.hard_links = 0;
    file_info.ea_size = 0;
}

fn file_attributes_from_attr(attr: &FileAttr) -> u32 {
    let mut attributes = match attr.kind {
        EntryKind::Directory => FILE_ATTRIBUTE_DIRECTORY.0,
        EntryKind::File => FILE_ATTRIBUTE_ARCHIVE.0,
    };

    if attr.readonly {
        attributes |= FILE_ATTRIBUTE_READONLY.0;
    }

    if attr.kind == EntryKind::File && attributes == 0 {
        FILE_ATTRIBUTE_NORMAL.0
    } else {
        attributes
    }
}

fn readonly_update_from_attributes(file_attributes: u32) -> Option<bool> {
    if file_attributes == INVALID_FILE_ATTRIBUTES {
        None
    } else {
        Some(file_attributes & FILE_ATTRIBUTE_READONLY.0 != 0)
    }
}

fn copy_security_descriptor(source: &[u8], target: Option<&mut [c_void]>) {
    if let Some(target) = target {
        let len = source.len().min(target.len());
        unsafe {
            std::ptr::copy_nonoverlapping(source.as_ptr(), target.as_mut_ptr().cast::<u8>(), len);
        }
    }
}

fn windows_time_from_wire(value: Option<FileTimeValue>) -> u64 {
    let Some(value) = value else {
        return 0;
    };

    if value.secs < -WINDOWS_EPOCH_OFFSET_SECS {
        return 0;
    }

    let unix_seconds = (value.secs + WINDOWS_EPOCH_OFFSET_SECS) as u64;
    unix_seconds
        .saturating_mul(WINDOWS_TICKS_PER_SECOND)
        .saturating_add((value.nanos as u64) / 100)
}

fn wire_time_from_windows(value: u64) -> Option<FileTimeValue> {
    if value == 0 || value == u64::MAX {
        return None;
    }

    let seconds = (value / WINDOWS_TICKS_PER_SECOND) as i64 - WINDOWS_EPOCH_OFFSET_SECS;
    let nanos = ((value % WINDOWS_TICKS_PER_SECOND) * 100) as u32;
    Some(FileTimeValue {
        secs: seconds,
        nanos,
    })
}

fn nt_path_to_relative(file_name: &U16CStr) -> String {
    file_name
        .to_string_lossy()
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

fn build_security_descriptor() -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let descriptor = widestring_from_str("O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;WD)");
    let mut raw_descriptor = PSECURITY_DESCRIPTOR::default();
    let mut len = 0u32;

    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(descriptor.as_ptr()),
            SDDL_REVISION_1,
            &mut raw_descriptor,
            Some(&mut len),
        )?;

        let bytes =
            std::slice::from_raw_parts(raw_descriptor.0.cast::<u8>(), len as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(raw_descriptor.0)));
        Ok(bytes)
    }
}

fn widestring_from_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
