use serde::{Deserialize, Serialize};
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTimeValue {
    pub secs: i64,
    pub nanos: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttr {
    pub kind: EntryKind,
    pub size: u64,
    pub allocated_size: u64,
    pub created: Option<FileTimeValue>,
    pub accessed: Option<FileTimeValue>,
    pub modified: Option<FileTimeValue>,
    pub changed: Option<FileTimeValue>,
    pub readonly: bool,
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub attr: FileAttr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicInfoUpdate {
    pub readonly: Option<bool>,
    pub creation_time: Option<FileTimeValue>,
    pub last_access_time: Option<FileTimeValue>,
    pub last_write_time: Option<FileTimeValue>,
    pub change_time: Option<FileTimeValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VolumeInfoData {
    pub total_size: u64,
    pub free_size: u64,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Stat {
        path: String,
    },
    Open {
        path: String,
    },
    Create {
        path: String,
        kind: EntryKind,
        file_attributes: u32,
        allocation_size: u64,
    },
    ReadDir {
        path: String,
    },
    Read {
        path: String,
        offset: u64,
        length: u32,
    },
    Write {
        path: String,
        offset: u64,
        data: Vec<u8>,
        write_to_eof: bool,
    },
    CanDelete {
        path: String,
        kind: EntryKind,
    },
    RemoveFile {
        path: String,
    },
    RemoveDir {
        path: String,
    },
    Rename {
        path: String,
        new_path: String,
        replace_if_exists: bool,
    },
    SetLen {
        path: String,
        size: u64,
        set_allocation_size: bool,
    },
    Flush {
        path: Option<String>,
    },
    SetBasicInfo {
        path: String,
        update: BasicInfoUpdate,
    },
    GetVolumeInfo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Response {
    Attr(FileAttr),
    DirEntries(Vec<DirEntry>),
    Data(Vec<u8>),
    WriteResult { written: u32, attr: FileAttr },
    VolumeInfo(VolumeInfoData),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    NotFound,
    AlreadyExists,
    PermissionDenied,
    InvalidInput,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    Unsupported,
    Unexpected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: ErrorCode,
    pub message: String,
}

pub type RpcResult<T> = Result<T, RpcError>;

impl RpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn to_io_error(&self) -> io::Error {
        let kind = match self.code {
            ErrorCode::NotFound => io::ErrorKind::NotFound,
            ErrorCode::AlreadyExists => io::ErrorKind::AlreadyExists,
            ErrorCode::PermissionDenied => io::ErrorKind::PermissionDenied,
            ErrorCode::InvalidInput => io::ErrorKind::InvalidInput,
            ErrorCode::NotDirectory => io::ErrorKind::NotADirectory,
            ErrorCode::IsDirectory => io::ErrorKind::IsADirectory,
            ErrorCode::DirectoryNotEmpty => io::ErrorKind::DirectoryNotEmpty,
            ErrorCode::Unsupported => io::ErrorKind::Unsupported,
            ErrorCode::Unexpected => io::ErrorKind::Other,
        };

        io::Error::new(kind, self.message.clone())
    }
}

impl From<io::Error> for RpcError {
    fn from(value: io::Error) -> Self {
        use io::ErrorKind;

        let code = match value.kind() {
            ErrorKind::NotFound => ErrorCode::NotFound,
            ErrorKind::AlreadyExists => ErrorCode::AlreadyExists,
            ErrorKind::PermissionDenied => ErrorCode::PermissionDenied,
            ErrorKind::InvalidInput => ErrorCode::InvalidInput,
            ErrorKind::NotADirectory => ErrorCode::NotDirectory,
            ErrorKind::IsADirectory => ErrorCode::IsDirectory,
            ErrorKind::DirectoryNotEmpty => ErrorCode::DirectoryNotEmpty,
            ErrorKind::Unsupported => ErrorCode::Unsupported,
            _ => ErrorCode::Unexpected,
        };

        Self::new(code, value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{EntryKind, FileAttr, Request, Response, RpcResult};

    #[test]
    fn roundtrips_request_and_response() {
        let request = Request::Create {
            path: "demo.txt".to_string(),
            kind: EntryKind::File,
            file_attributes: 0,
            allocation_size: 32,
        };
        let encoded = bincode::serde::encode_to_vec(&request, bincode::config::standard()).unwrap();
        let (decoded, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
        assert_eq!(request, decoded);

        let response: RpcResult<Response> = Ok(Response::Attr(FileAttr {
            kind: EntryKind::File,
            size: 12,
            allocated_size: 4096,
            created: None,
            accessed: None,
            modified: None,
            changed: None,
            readonly: false,
            mode: Some(0o644),
        }));

        let encoded =
            bincode::serde::encode_to_vec(&response, bincode::config::standard()).unwrap();
        let (decoded, _) =
            bincode::serde::decode_from_slice(&encoded, bincode::config::standard()).unwrap();
        assert_eq!(response, decoded);
    }
}
