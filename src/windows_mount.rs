use crate::{MountArgs, UpArgs};

pub fn run_mount(_args: MountArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("Windows mount support has not been implemented yet".into())
}

pub fn run_up(_args: UpArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    Err("Windows orchestration has not been implemented yet".into())
}
