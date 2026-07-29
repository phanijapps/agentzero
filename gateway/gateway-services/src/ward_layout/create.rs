use std::collections::BTreeMap;
#[cfg(not(target_os = "linux"))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use thiserror::Error;

use agent_primitives::WardArchetypeId;

use super::{
    load_ward_archetype_bundle, CompiledWardLayout, LayoutError, NodeFormat, NodeKind, RuleNode,
    WardStarterFile, MAX_WARD_ARCHETYPE_STARTER_FILE_BYTES, WARD_AGENT_TEMPLATE_MAX_BYTES,
};
use crate::VaultPaths;

/// Publish a fully-resolved directory tree beneath a real ward directory.
/// Linux uses directory-relative, no-follow operations and an atomic
/// no-replace rename. Callers remain responsible for resolving `files` from
/// the active template before invoking this filesystem primitive.
pub fn publish_tree_no_replace(
    wards_root: &Path,
    ward_name: &str,
    parent_components: &[String],
    name: &str,
    files: &[(PathBuf, Option<Vec<u8>>)],
) -> Result<(), WardCreateError> {
    validate_ward_id(name)?;
    validate_ward_id(ward_name)?;
    #[cfg(target_os = "linux")]
    {
        static PUBLICATION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _publication = PUBLICATION_LOCK
            .lock()
            .map_err(|_| WardCreateError::Invalid("publication lock poisoned".into()))?;
        let metadata = fs::symlink_metadata(wards_root)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(WardCreateError::Invalid(
                "wards root must be a real directory".into(),
            ));
        }
        let wards = open_directory_path(wards_root)?;
        let opened = wards.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
                return Err(WardCreateError::Invalid(
                    "wards root changed while opening".into(),
                ));
            }
        }
        let mut parent = open_directory_at(&wards, ward_name)?;
        for component in parent_components {
            validate_ward_id(component)?;
            parent = open_directory_at(&parent, component)?;
        }
        ensure_no_casefold_collision_at(&parent, name)?;
        let staging_name = format!(".{name}.zbot-staging-{}", uuid::Uuid::new_v4());
        mkdir_at(&parent, &staging_name)?;
        let staging = open_directory_at(&parent, &staging_name)?;
        let result = (|| {
            for (relative, bytes) in files {
                let mut components = relative.components().peekable();
                let mut directory = staging.try_clone()?;
                while let Some(component) = components.next() {
                    let std::path::Component::Normal(component) = component else {
                        return Err(WardCreateError::Invalid("unsafe tree path".into()));
                    };
                    let component = component.to_str().ok_or_else(|| {
                        WardCreateError::Invalid("tree path must be UTF-8".into())
                    })?;
                    if components.peek().is_none() {
                        if let Some(bytes) = bytes {
                            write_new_at(&directory, component, bytes)?;
                        } else {
                            match mkdir_at(&directory, component) {
                                Ok(()) => {}
                                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                                }
                                Err(error) => return Err(error.into()),
                            }
                        }
                    } else {
                        match mkdir_at(&directory, component) {
                            Ok(()) => {}
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                            Err(error) => return Err(error.into()),
                        }
                        directory = open_directory_at(&directory, component)?;
                    }
                }
            }
            rename_no_replace_at(&parent, &staging_name, &parent, name)?;
            Ok(())
        })();
        if let Err(error) = result {
            use std::os::fd::AsRawFd;
            let anchored = PathBuf::from(format!(
                "/proc/self/fd/{}/{}",
                parent.as_raw_fd(),
                staging_name
            ));
            return match fs::remove_dir_all(anchored) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WardCreateError::Invalid(format!(
                    "tree publication failed and staging cleanup failed: {cleanup}"
                ))),
            };
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (wards_root, parent_components, files);
        Err(WardCreateError::Invalid(
            "secure concept publication is unavailable on this platform".into(),
        ))
    }
}

/// Remove a Ward that was just published when a required follow-up record
/// cannot be committed. The Ward is first moved to an unaddressable rollback
/// name so callers never leave the requested Ward id partially committed.
pub fn rollback_created_ward(paths: &VaultPaths, ward_id: &str) -> Result<(), WardCreateError> {
    validate_ward_id(ward_id)?;
    let wards_root = paths.wards_dir();
    let rollback_name = format!(".{ward_id}.rollback-{}", uuid::Uuid::new_v4());
    #[cfg(target_os = "linux")]
    {
        let metadata = fs::symlink_metadata(&wards_root)?;
        let root = open_directory_path(&wards_root)?;
        let opened = root.metadata()?;
        use std::os::unix::fs::MetadataExt;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.dev() != opened.dev()
            || metadata.ino() != opened.ino()
        {
            return Err(WardCreateError::Invalid(
                "configured wards root changed while it was opened".into(),
            ));
        }
        rename_at(&root, ward_id, &root, &rollback_name)?;
        let anchored = PathBuf::from(format!(
            "/proc/self/fd/{}/{}",
            std::os::fd::AsRawFd::as_raw_fd(&root),
            rollback_name
        ));
        fs::remove_dir_all(anchored)?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let rollback = wards_root.join(&rollback_name);
        fs::rename(paths.ward_dir(ward_id), &rollback)?;
        fs::remove_dir_all(rollback)?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn ensure_no_casefold_collision_at(parent: &File, name: &str) -> Result<(), WardCreateError> {
    use std::os::fd::AsRawFd;

    let directory = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()));
    for entry in fs::read_dir(directory)? {
        let existing = entry?.file_name();
        let existing = existing
            .to_str()
            .ok_or_else(|| WardCreateError::Invalid("non-UTF-8 sibling".into()))?;
        if existing.eq_ignore_ascii_case(name) {
            return Err(WardCreateError::Invalid(
                "destination collides with an existing sibling".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedWard {
    pub path: PathBuf,
    pub snapshot_digest: String,
    pub archetype: WardArchetypeId,
}

#[derive(Debug, Error)]
pub enum WardCreateError {
    #[error(transparent)]
    Layout(#[from] LayoutError),
    #[error("ward creation failed: {0}")]
    Invalid(String),
    #[error("ward creation I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Compatibility entry point for callers that do not yet select an
/// archetype. The singular files are consulted only by one-time registry
/// seeding; materialization always loads the generic registry bundle.
pub fn create_ward_from_template(
    paths: &VaultPaths,
    ward_id: &str,
) -> Result<CreatedWard, WardCreateError> {
    super::seed_default_ward_archetypes(paths)?;
    create_ward_from_archetype(paths, ward_id, None)
}

pub fn create_ward_from_archetype(
    paths: &VaultPaths,
    ward_id: &str,
    archetype: Option<WardArchetypeId>,
) -> Result<CreatedWard, WardCreateError> {
    validate_new_ward_id(ward_id)?;
    let archetype = archetype.unwrap_or_default();
    let bundle = load_ward_archetype_bundle(paths, archetype)?;
    let layout = CompiledWardLayout::compile(&bundle.layout.document)
        .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
    validate_ward_identity_against_layout(ward_id, &layout)?;
    let agent_doctrine = if root_requires_agent_instructions(&layout)? {
        Some(render_agent_template(&bundle.doctrine, ward_id)?)
    } else {
        None
    };
    let starters = starter_map(&bundle.starters, ward_id)?;
    let materialization = Materialization {
        layout: &layout,
        ward_id: Some(ward_id),
        agent_doctrine: agent_doctrine.as_deref(),
        starters: &starters,
    };
    let wards_root = paths.wards_dir();
    let root_metadata = fs::symlink_metadata(&wards_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(WardCreateError::Invalid(
            "configured wards root must be a real directory".into(),
        ));
    }
    ensure_no_casefold_collision_path(&wards_root, ward_id)?;

    let ward = paths.ward_dir(ward_id);
    #[cfg(target_os = "linux")]
    {
        create_linux(
            &wards_root,
            &ward,
            ward_id,
            &bundle.layout.bytes,
            bundle.layout.digest,
            &materialization,
            archetype,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        create_portable(
            &ward,
            &bundle.layout.bytes,
            bundle.layout.digest,
            &materialization,
            archetype,
        )
    }
}

fn starter_map(
    starters: &[WardStarterFile],
    ward_id: &str,
) -> Result<BTreeMap<PathBuf, String>, WardCreateError> {
    starters
        .iter()
        .map(|starter| {
            let destination = if starter.relative_path == Path::new("{ward}.md") {
                PathBuf::from(format!("{ward_id}.md"))
            } else {
                starter.relative_path.clone()
            };
            let content = render_template(
                &starter.content,
                ward_id,
                MAX_WARD_ARCHETYPE_STARTER_FILE_BYTES,
                "starter",
            )?;
            Ok((destination, content))
        })
        .collect()
}

struct Materialization<'a> {
    layout: &'a CompiledWardLayout,
    ward_id: Option<&'a str>,
    agent_doctrine: Option<&'a str>,
    starters: &'a BTreeMap<PathBuf, String>,
}

#[cfg(not(target_os = "linux"))]
fn create_portable(
    ward: &Path,
    template_bytes: &[u8],
    digest: String,
    materialization: &Materialization<'_>,
    archetype: WardArchetypeId,
) -> Result<CreatedWard, WardCreateError> {
    // Non-Linux targets do not have `renameat2(RENAME_NOREPLACE)`. Reserve
    // the final directory with `create_dir` instead: it is exclusive and
    // never overwrites an existing ward, unlike check-then-rename staging.
    fs::create_dir(ward).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            WardCreateError::Invalid("ward already exists".into())
        } else {
            WardCreateError::Io(error)
        }
    })?;
    let result = (|| {
        write_new(&ward.join("ward-conf.yaml"), template_bytes)?;
        materialize_children(
            ward,
            &materialization.layout.root,
            true,
            Path::new(""),
            materialization,
        )?;
        Ok(CreatedWard {
            path: ward.to_path_buf(),
            snapshot_digest: digest,
            archetype,
        })
    })();
    match result {
        Ok(created) => Ok(created),
        Err(error) => match fs::remove_dir_all(ward) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(WardCreateError::Invalid(format!(
                "{error}; failed to clean incomplete ward: {cleanup}"
            ))),
        },
    }
}

#[cfg(target_os = "linux")]
fn create_linux(
    wards_root: &Path,
    ward: &Path,
    ward_id: &str,
    template_bytes: &[u8],
    digest: String,
    materialization: &Materialization<'_>,
    archetype: WardArchetypeId,
) -> Result<CreatedWard, WardCreateError> {
    use std::os::unix::fs::MetadataExt;

    let before = fs::symlink_metadata(wards_root)?;
    let root = File::open(wards_root)?;
    let opened = root.metadata()?;
    if before.dev() != opened.dev()
        || before.ino() != opened.ino()
        || !opened.is_dir()
        || before.file_type().is_symlink()
    {
        return Err(WardCreateError::Invalid(
            "configured wards root changed while it was opened".into(),
        ));
    }
    ensure_no_casefold_collision_at(&root, ward_id)?;
    let staging_name = format!(".{ward_id}.staging-{}", uuid::Uuid::new_v4());
    mkdir_at(&root, &staging_name).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            WardCreateError::Invalid("ward already exists".into())
        } else {
            WardCreateError::Io(error)
        }
    })?;
    let result = (|| {
        let staging = open_directory_at(&root, &staging_name)?;
        write_new_at(&staging, "ward-conf.yaml", template_bytes)?;
        materialize_children_at(
            &staging,
            &materialization.layout.root,
            true,
            Path::new(""),
            materialization,
        )?;
        rename_no_replace_at(&root, &staging_name, &root, ward_id)?;
        Ok(CreatedWard {
            path: ward.to_path_buf(),
            snapshot_digest: digest,
            archetype,
        })
    })();
    match result {
        Ok(created) => Ok(created),
        Err(error) => {
            use std::os::fd::AsRawFd;
            let anchored = PathBuf::from(format!(
                "/proc/self/fd/{}/{}",
                root.as_raw_fd(),
                staging_name
            ));
            match fs::remove_dir_all(anchored) {
                Ok(()) => Err(error),
                Err(cleanup) => Err(WardCreateError::Invalid(format!(
                    "{error}; failed to clean staging ward: {cleanup}"
                ))),
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn open_directory_path(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    const O_DIRECTORY: i32 = 0o200000;
    const O_NOFOLLOW: i32 = 0o400000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_DIRECTORY | O_NOFOLLOW)
        .open(path)
}

#[cfg(target_os = "linux")]
fn mkdir_at(parent: &File, name: &str) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn mkdirat(dirfd: i32, pathname: *const std::ffi::c_char, mode: u32) -> i32;
    }
    let name = CString::new(name).map_err(|_| std::io::Error::other("invalid path component"))?;
    // SAFETY: `name` is NUL-terminated and `parent` owns a live directory fd.
    if unsafe { mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o755) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn open_directory_at(parent: &File, name: &str) -> std::io::Result<File> {
    open_at(parent, name, 0o200000 | 0o400000 | 0o2000000, 0)
}

#[cfg(target_os = "linux")]
fn write_new_at(parent: &File, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = open_at(
        parent,
        name,
        0o1 | 0o100 | 0o200 | 0o400000 | 0o2000000,
        0o644,
    )?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(target_os = "linux")]
fn open_at(parent: &File, name: &str, flags: i32, mode: u32) -> std::io::Result<File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};

    unsafe extern "C" {
        fn openat(dirfd: i32, pathname: *const std::ffi::c_char, flags: i32, ...) -> i32;
    }
    let name = CString::new(name).map_err(|_| std::io::Error::other("invalid path component"))?;
    // SAFETY: `name` is NUL-terminated, `parent` owns a live directory fd,
    // and the mode argument is supplied for every call.
    let fd = unsafe { openat(parent.as_raw_fd(), name.as_ptr(), flags, mode) };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: `openat` returned a fresh descriptor now owned by `File`.
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(target_os = "linux")]
fn rename_no_replace_at(
    source_parent: &File,
    source_name: &str,
    destination_parent: &File,
    destination_name: &str,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn renameat2(
            olddirfd: i32,
            oldpath: *const std::ffi::c_char,
            newdirfd: i32,
            newpath: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let source =
        CString::new(source_name).map_err(|_| std::io::Error::other("invalid path component"))?;
    let destination = CString::new(destination_name)
        .map_err(|_| std::io::Error::other("invalid path component"))?;
    // SAFETY: both names are NUL-terminated components and the parent handles
    // remain open for the entire no-clobber publication.
    if unsafe {
        renameat2(
            source_parent.as_raw_fd(),
            source.as_ptr(),
            destination_parent.as_raw_fd(),
            destination.as_ptr(),
            1, // RENAME_NOREPLACE
        )
    } == -1
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn rename_at(
    old_parent: &File,
    old_name: &str,
    new_parent: &File,
    new_name: &str,
) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn renameat(
            olddirfd: i32,
            oldpath: *const std::ffi::c_char,
            newdirfd: i32,
            newpath: *const std::ffi::c_char,
        ) -> i32;
    }
    let old_name = CString::new(old_name).map_err(|_| std::io::Error::other("invalid old name"))?;
    let new_name = CString::new(new_name).map_err(|_| std::io::Error::other("invalid new name"))?;
    // SAFETY: both names are NUL-terminated and both parent descriptors are live.
    if unsafe {
        renameat(
            old_parent.as_raw_fd(),
            old_name.as_ptr(),
            new_parent.as_raw_fd(),
            new_name.as_ptr(),
        )
    } == -1
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn materialize_children_at(
    parent: &File,
    node: &RuleNode,
    is_ward_root: bool,
    relative_parent: &Path,
    materialization: &Materialization<'_>,
) -> Result<(), WardCreateError> {
    let effective = materialization
        .layout
        .effective(node)
        .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
    for child in &effective.children {
        let Some(pattern) = child.match_pattern.as_deref() else {
            continue;
        };
        if !child.required
            || child.repeat
            || child.reference.is_some()
            || pattern.contains('*')
            || pattern.contains("{name}")
        {
            continue;
        }
        let component = materialized_component(pattern, is_ward_root, materialization.ward_id)?;
        let effective_child = materialization
            .layout
            .effective(child)
            .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
        let relative = relative_parent.join(&component);
        match effective_child.kind.expect("validated rule kind") {
            NodeKind::Directory => {
                mkdir_at(parent, &component)?;
                let child_dir = open_directory_at(parent, &component)?;
                materialize_children_at(
                    &child_dir,
                    effective_child,
                    false,
                    &relative,
                    materialization,
                )?;
            }
            NodeKind::File => {
                if let Some(content) = materialization.starters.get(&relative) {
                    write_new_at(parent, &component, content.as_bytes())?;
                } else {
                    let content = scaffold_content(
                        child,
                        effective_child.format,
                        is_ward_root,
                        materialization.ward_id,
                        materialization.agent_doctrine,
                    );
                    write_new_at(parent, &component, content.as_bytes())?;
                }
            }
        }
    }
    Ok(())
}

fn validate_ward_id(value: &str) -> Result<(), WardCreateError> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(WardCreateError::Invalid(
            "ward id must be 1-64 ASCII letters, numbers, hyphens, or underscores".into(),
        ));
    }
    Ok(())
}

fn validate_new_ward_id(value: &str) -> Result<(), WardCreateError> {
    validate_ward_id(value)?;
    if !value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
    }) || matches!(
        value,
        "agents" | "index" | "log" | "pages" | "sources" | "ward-conf"
    ) {
        return Err(WardCreateError::Invalid(
            "new ward id must be a lowercase ASCII slug that does not claim a reserved root identity"
                .into(),
        ));
    }
    Ok(())
}

fn validate_ward_identity_against_layout(
    ward_id: &str,
    layout: &CompiledWardLayout,
) -> Result<(), WardCreateError> {
    let root = layout
        .effective(&layout.root)
        .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
    for child in &root.children {
        let effective = layout
            .effective(child)
            .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
        let Some(pattern) = child.match_pattern.as_deref() else {
            continue;
        };
        if pattern.contains(['*', '{', '}']) {
            continue;
        }
        let identity = if effective.kind == Some(NodeKind::File) {
            Path::new(pattern)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(pattern)
        } else {
            pattern
        };
        if identity.eq_ignore_ascii_case(ward_id) {
            return Err(WardCreateError::Invalid(
                "new ward id claims a reserved archetype root identity".into(),
            ));
        }
    }
    Ok(())
}

fn ensure_no_casefold_collision_path(parent: &Path, name: &str) -> Result<(), WardCreateError> {
    for entry in fs::read_dir(parent)? {
        let existing = entry?.file_name();
        let existing = existing
            .to_str()
            .ok_or_else(|| WardCreateError::Invalid("non-UTF-8 Ward sibling".into()))?;
        if existing.eq_ignore_ascii_case(name) {
            return Err(WardCreateError::Invalid(
                "Ward destination collides with an existing sibling".into(),
            ));
        }
    }
    Ok(())
}

fn materialized_component(
    pattern: &str,
    is_ward_root: bool,
    ward_id: Option<&str>,
) -> Result<String, WardCreateError> {
    if !pattern.contains("{ward}") {
        return Ok(pattern.to_owned());
    }
    if !is_ward_root {
        return Err(WardCreateError::Invalid(
            "canonical ward placeholder escaped the Ward root".into(),
        ));
    }
    let ward_id = ward_id.ok_or_else(|| {
        WardCreateError::Invalid("canonical ward placeholder requires a Ward id".into())
    })?;
    Ok(pattern.replace("{ward}", ward_id))
}

#[cfg(not(target_os = "linux"))]
fn materialize_children(
    parent: &Path,
    node: &RuleNode,
    is_ward_root: bool,
    relative_parent: &Path,
    materialization: &Materialization<'_>,
) -> Result<(), WardCreateError> {
    let effective = materialization
        .layout
        .effective(node)
        .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
    for child in &effective.children {
        let Some(pattern) = child.match_pattern.as_deref() else {
            continue;
        };
        if !child.required
            || child.repeat
            || child.reference.is_some()
            || pattern.contains('*')
            || pattern.contains("{name}")
        {
            continue;
        }
        let component = materialized_component(pattern, is_ward_root, materialization.ward_id)?;
        let effective_child = materialization
            .layout
            .effective(child)
            .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
        let destination = parent.join(&component);
        let relative = relative_parent.join(&component);
        match effective_child.kind.expect("validated rule kind") {
            NodeKind::Directory => {
                fs::create_dir(&destination)?;
                materialize_children(
                    &destination,
                    effective_child,
                    false,
                    &relative,
                    materialization,
                )?;
            }
            NodeKind::File => {
                if let Some(content) = materialization.starters.get(&relative) {
                    write_new(&destination, content.as_bytes())?;
                } else {
                    let content = scaffold_content(
                        child,
                        effective_child.format,
                        is_ward_root,
                        materialization.ward_id,
                        materialization.agent_doctrine,
                    );
                    write_new(&destination, content.as_bytes())?;
                }
            }
        }
    }
    Ok(())
}

fn scaffold_content(
    node: &RuleNode,
    format: Option<NodeFormat>,
    is_ward_root: bool,
    ward_id: Option<&str>,
    agent_doctrine: Option<&str>,
) -> String {
    let title = node
        .id
        .as_deref()
        .unwrap_or("document")
        .replace(['-', '_'], " ");
    match format {
        Some(NodeFormat::OkfV01) => {
            let tags = ward_id
                .map(|ward_id| format!("tags:\n  - {}\n", yaml_string(ward_id)))
                .unwrap_or_default();
            format!(
                "---\ntype: {}\ntitle: {}\n{}---\n\n# {}\n",
                yaml_string(node.id.as_deref().unwrap_or("document")),
                yaml_string(&title),
                tags,
                title
            )
        }
        Some(NodeFormat::Markdown)
            if is_ward_root && node.match_pattern.as_deref() == Some("AGENTS.md") =>
        {
            agent_doctrine.unwrap_or_default().to_owned()
        }
        Some(NodeFormat::Markdown) => format!("# {title}\n"),
        Some(NodeFormat::Raw) | None => String::new(),
    }
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

fn root_requires_agent_instructions(layout: &CompiledWardLayout) -> Result<bool, WardCreateError> {
    let root = layout
        .effective(&layout.root)
        .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
    for child in &root.children {
        if child.required
            && !child.repeat
            && child.reference.is_none()
            && child.match_pattern.as_deref() == Some("AGENTS.md")
        {
            let effective = layout
                .effective(child)
                .map_err(|error| WardCreateError::Invalid(error.to_string()))?;
            return Ok(effective.kind == Some(NodeKind::File)
                && effective.format == Some(NodeFormat::Markdown));
        }
    }
    Ok(false)
}

fn render_agent_template(template: &str, ward_id: &str) -> Result<String, WardCreateError> {
    render_template(
        template,
        ward_id,
        WARD_AGENT_TEMPLATE_MAX_BYTES,
        "agent template",
    )
}

fn render_template(
    template: &str,
    ward_id: &str,
    max_bytes: usize,
    label: &str,
) -> Result<String, WardCreateError> {
    let rendered = template
        .replace("{{ward_id}}", ward_id)
        .replace("{{display_name}}", &ward_display_name(ward_id));
    if rendered.len() > max_bytes {
        return Err(WardCreateError::Invalid(format!(
            "rendered ward {label} exceeds the byte limit"
        )));
    }
    Ok(rendered)
}

fn ward_display_name(ward: &str) -> String {
    let display_name = ward
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    if display_name.is_empty() {
        format!("Ward ({ward})")
    } else {
        display_name
    }
}

#[cfg(not(target_os = "linux"))]
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ward_layout::{lint_ward, load_ward_layout, seed_default_ward_archetypes};
    use tempfile::tempdir;

    // STUB: AC5
    #[test]
    fn new_ward_ids_require_lowercase_canonical_slugs() {
        assert!(validate_new_ward_id("financial-analysis").is_ok());
        assert!(validate_new_ward_id("Financial-Analysis").is_err());
        for reserved in ["agents", "index", "log", "pages", "sources", "ward-conf"] {
            assert!(validate_new_ward_id(reserved).is_err(), "{reserved}");
        }
    }

    #[test]
    fn new_ward_ids_reject_casefold_siblings_and_archetype_root_identities() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        fs::create_dir(paths.ward_dir("Financial-Analysis")).unwrap();
        assert!(create_ward_from_archetype(
            &paths,
            "financial-analysis",
            Some(WardArchetypeId::Generic)
        )
        .is_err());
        assert!(!paths.ward_dir("financial-analysis").exists());

        for (archetype, reserved) in [
            (WardArchetypeId::Coding, "src"),
            (WardArchetypeId::Documentation, "topics"),
            (WardArchetypeId::Journal, "entries"),
            (WardArchetypeId::Ebook, "books"),
            (WardArchetypeId::Research, "subjects"),
            (WardArchetypeId::News, "archive"),
        ] {
            assert!(
                create_ward_from_archetype(&paths, reserved, Some(archetype)).is_err(),
                "{archetype}: {reserved}"
            );
            assert!(
                !paths.ward_dir(reserved).exists(),
                "{archetype}: {reserved}"
            );
        }
    }

    // STUB: AC1, AC2, AC3
    #[test]
    fn fresh_generic_ward_has_exact_llm_wiki_root() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        let bundle = paths.ward_archetype_bundle(WardArchetypeId::Generic);
        std::fs::create_dir_all(bundle.join("starters")).unwrap();
        std::fs::write(
            bundle.join("ward-conf.yaml"),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: canonical, match: '{ward}.md', kind: file, format: markdown }\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n    - { id: log, match: log.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        std::fs::write(bundle.join("ward-agent.md"), "# {{display_name}} Agent\n").unwrap();
        std::fs::write(bundle.join("starters/canonical.md"), "# {{display_name}}\n").unwrap();
        std::fs::write(bundle.join("starters/log.md"), "# Log\n").unwrap();

        let created = create_ward_from_archetype(
            &paths,
            "financial-analysis",
            Some(WardArchetypeId::Generic),
        )
        .unwrap();
        let mut names = std::fs::read_dir(&created.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                "AGENTS.md",
                "financial-analysis.md",
                "log.md",
                "ward-conf.yaml"
            ]
        );
    }

    #[test]
    fn selected_coding_bundle_controls_snapshot_doctrine_starters_and_tree() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        let created =
            create_ward_from_archetype(&paths, "compiler-lab", Some(WardArchetypeId::Coding))
                .unwrap();

        assert_eq!(created.archetype, WardArchetypeId::Coding);
        assert_eq!(
            fs::read(created.path.join("ward-conf.yaml")).unwrap(),
            fs::read(
                paths
                    .ward_archetype_bundle(WardArchetypeId::Coding)
                    .join("ward-conf.yaml")
            )
            .unwrap()
        );
        assert_eq!(
            fs::read_to_string(created.path.join("compiler-lab.md")).unwrap(),
            include_str!("../../../templates/wards/coding/starters/canonical.md")
                .replace("{{display_name}}", "Compiler Lab")
        );
        let doctrine = fs::read_to_string(created.path.join("AGENTS.md")).unwrap();
        assert!(doctrine.contains("# Compiler Lab Coding Ward Agent"));
        assert!(doctrine.contains("creating source"));
        for directory in ["src", "tests", "docs", "scripts", "artifacts", ".zbot"] {
            assert!(!created.path.join(directory).exists(), "{directory}");
        }
        assert!(!created.path.join("notes").exists());
        assert!(!created.path.join("sources").exists());
        let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
        assert!(lint_ward(&created.path, &snapshot).valid);
    }

    #[test]
    fn all_archetypes_materialize_compact_complete_bundles() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        for (id, ward) in [
            (WardArchetypeId::Generic, "general-work"),
            (WardArchetypeId::Coding, "software-work"),
            (WardArchetypeId::Documentation, "docs-work"),
            (WardArchetypeId::Journal, "journal-work"),
            (WardArchetypeId::Ebook, "ebook-work"),
            (WardArchetypeId::Research, "research-work"),
            (WardArchetypeId::News, "news-work"),
        ] {
            let bundle = load_ward_archetype_bundle(&paths, id).unwrap();
            let created = create_ward_from_archetype(&paths, ward, Some(id)).unwrap();
            assert_eq!(created.archetype, id);
            assert_eq!(created.snapshot_digest, bundle.layout.digest);
            assert_eq!(
                fs::read(created.path.join("ward-conf.yaml")).unwrap(),
                bundle.layout.bytes
            );
            let mut entries = fs::read_dir(&created.path)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            entries.sort();
            let mut expected = vec![
                "AGENTS.md".to_owned(),
                format!("{ward}.md"),
                "log.md".to_owned(),
                "ward-conf.yaml".to_owned(),
            ];
            expected.sort();
            assert_eq!(entries, expected, "{id}");
            let canonical = fs::read_to_string(created.path.join(format!("{ward}.md"))).unwrap();
            assert!(!canonical.starts_with("---"), "{id}");
            assert!(canonical.contains("[[#Concepts|Concepts]]"), "{id}");
            assert!(canonical.contains("[[#Tags|Tags]]"), "{id}");
            assert!(fs::read_to_string(created.path.join("log.md"))
                .unwrap()
                .contains("## [YYYY-MM-DD] <operation> | <subject>"));
            assert!(
                !fs::read_to_string(created.path.join("log.md"))
                    .unwrap()
                    .lines()
                    .any(|line| line.starts_with("## [20")),
                "{id}"
            );
            let layout = String::from_utf8(bundle.layout.bytes.clone()).unwrap();
            for required in [
                "match: pages",
                "match: sources",
                "match: .zbot",
                "match: plan.md",
            ] {
                assert!(layout.contains(required), "{id}: missing {required}");
            }
            assert!(!layout.contains("okf-v0.1"), "{id}");
            assert!(!layout.contains("tasks/index.md"), "{id}");
            let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
            assert!(lint_ward(&created.path, &snapshot).valid, "{id}");
        }
    }

    #[test]
    fn journal_bundle_routes_daily_entries_without_eager_scaffolding() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        let created =
            create_ward_from_archetype(&paths, "daily-life", Some(WardArchetypeId::Journal))
                .unwrap();
        let mut root_entries = fs::read_dir(&created.path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        root_entries.sort();
        assert_eq!(
            root_entries,
            ["AGENTS.md", "daily-life.md", "log.md", "ward-conf.yaml"],
            "journal creation must remain compact"
        );

        let entries = created.path.join("entries");
        fs::create_dir(&entries).unwrap();
        fs::write(entries.join("compiled.md"), "# Compiled days\n").unwrap();
        let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
        let flat_report = lint_ward(&created.path, &snapshot);
        assert!(
            !flat_report.valid,
            "Markdown directly under entries/ must not lint as a daily journal"
        );
        assert!(flat_report.findings.iter().any(|finding| {
            finding.code == "undeclared_markdown" && finding.path == "entries/compiled.md"
        }));

        fs::remove_file(entries.join("compiled.md")).unwrap();
        fs::create_dir(entries.join("2026")).unwrap();
        fs::write(entries.join("2026/2026-07-27.md"), "# 2026-07-27\n").unwrap();
        assert!(
            lint_ward(&created.path, &snapshot).valid,
            "one dated Markdown file beneath its year must lint"
        );

        let doctrine = fs::read_to_string(created.path.join("AGENTS.md")).unwrap();
        let canonical = fs::read_to_string(created.path.join("daily-life.md")).unwrap();
        for content in [&doctrine, &canonical] {
            assert!(content.contains("entries/YYYY/YYYY-MM-DD.md"));
            assert!(content.contains("one file per source day"));
        }
        assert!(doctrine.contains("unless the user explicitly requests a compilation"));
    }

    #[test]
    fn missing_selected_bundle_fails_without_publishing_a_ward() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();

        let result = create_ward_from_archetype(&paths, "no-news", Some(WardArchetypeId::News));

        assert!(result.is_err());
        assert!(!paths.ward_dir("no-news").exists());
        assert!(fs::read_dir(paths.wards_dir()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("staging")
        }));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn portable_creation_snapshots_selected_bundle() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        for archetype in WardArchetypeId::ALL {
            let ward = format!("portable-{archetype}");
            let bundle = load_ward_archetype_bundle(&paths, archetype).unwrap();
            let created = create_ward_from_archetype(&paths, &ward, Some(archetype)).unwrap();
            assert_eq!(created.archetype, archetype);
            assert_eq!(created.snapshot_digest, bundle.layout.digest);
            assert_eq!(
                fs::read(created.path.join("ward-conf.yaml")).unwrap(),
                bundle.layout.bytes
            );
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn portable_failure_cleans_destination() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        seed_default_ward_archetypes(&paths).unwrap();

        let existing = paths.ward_dir("existing");
        fs::create_dir(&existing).unwrap();
        fs::write(existing.join("keep.txt"), "keep").unwrap();
        assert!(
            create_ward_from_archetype(&paths, "existing", Some(WardArchetypeId::Coding)).is_err()
        );
        assert_eq!(
            fs::read_to_string(existing.join("keep.txt")).unwrap(),
            "keep"
        );

        fs::remove_file(
            paths
                .ward_archetype_bundle(WardArchetypeId::Coding)
                .join("ward-agent.md"),
        )
        .unwrap();
        assert!(
            create_ward_from_archetype(&paths, "incomplete", Some(WardArchetypeId::Coding))
                .is_err()
        );
        assert!(!paths.ward_dir("incomplete").exists());
    }

    #[test]
    fn creates_only_required_literal_nodes_from_a_fluid_template() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        let yaml = "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: landing, match: home.md, kind: file, format: markdown }\n    - { id: optional, match: spec.md, kind: file, required: false, format: markdown }\n    - { id: assets, match: assets, kind: directory }\n";
        fs::write(paths.ward_layout_template(), yaml).unwrap();

        let created = create_ward_from_template(&paths, "fluid").unwrap();
        assert!(created.path.join("home.md").is_file());
        assert!(created.path.join("assets").is_dir());
        assert!(!created.path.join("spec.md").exists());
        assert_eq!(
            fs::read(created.path.join("ward-conf.yaml")).unwrap(),
            yaml.as_bytes()
        );
        let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
        assert!(lint_ward(&created.path, &snapshot).valid);
    }

    #[test]
    fn fresh_ward_okf_scaffold_has_type_title_tags_and_lints() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: 'true', match: index.md, kind: file, format: okf-v0.1 }\n",
        )
        .unwrap();

        let created = create_ward_from_template(&paths, "null").unwrap();
        let index = fs::read_to_string(created.path.join("index.md")).unwrap();
        let frontmatter = index
            .strip_prefix("---\n")
            .and_then(|value| value.split_once("\n---\n"))
            .map(|(frontmatter, _)| frontmatter)
            .unwrap();
        let metadata: serde_yaml::Value = serde_yaml::from_str(frontmatter).unwrap();
        assert_eq!(metadata["type"].as_str(), Some("true"));
        assert_eq!(metadata["title"].as_str(), Some("true"));
        assert_eq!(metadata["tags"][0].as_str(), Some("null"));

        let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
        assert!(lint_ward(&created.path, &snapshot).valid);
    }

    #[test]
    fn default_template_scaffolds_compact_llm_wiki_root() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        super::super::seed_default_ward_layout_template(&paths).unwrap();
        super::super::seed_default_ward_agent_template(&paths).unwrap();

        let created = create_ward_from_template(&paths, "minimal-concepts").unwrap();
        assert!(created.path.join("minimal-concepts.md").is_file());
        assert!(created.path.join("log.md").is_file());
        assert!(!created.path.join("pages").exists());
        assert!(!created.path.join("sources").exists());

        let snapshot = load_ward_layout(&created.path.join("ward-conf.yaml")).unwrap();
        assert!(lint_ward(&created.path, &snapshot).valid);
    }

    #[test]
    fn scaffolds_generic_agent_instructions_without_assuming_directory_roles() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n    - { id: notes, match: notes.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        super::super::seed_default_ward_agent_template(&paths).unwrap();

        let created = create_ward_from_template(&paths, "with-instructions").unwrap();
        let instructions = fs::read_to_string(created.path.join("AGENTS.md")).unwrap();

        assert!(instructions.contains("# With Instructions Ward Agent"));
        assert!(instructions.contains("## Identity"));
        assert!(instructions.contains("persistent Ward agent"));
        assert!(instructions.contains("## Persona"));
        assert!(instructions.contains("evidence"));
        assert!(instructions.contains("uncertainty"));
        assert!(instructions.contains("## Purpose and Scope"));
        assert!(instructions.contains("## Operating Principles"));
        assert!(instructions.contains("## Knowledge Navigation"));
        assert!(instructions.contains("## Workflow"));
        assert!(instructions.contains("## Self-Maintenance"));
        assert!(instructions.contains("Propose durable doctrine changes in your handoff"));
        assert!(instructions.contains("only with explicit user direction"));
        assert!(instructions.contains("Never delete or rewrite existing persona text"));
        assert!(instructions.contains("## Handoff"));
        assert!(instructions.contains("active ward template"));
        assert!(instructions.contains("user-editable"));
        assert!(!instructions.contains("src/"));
        assert!(!instructions.contains("data/"));
        assert!(!instructions.contains("reports/"));
        assert!(!instructions.contains("output/"));
        assert_eq!(
            fs::read_to_string(created.path.join("notes.md")).unwrap(),
            "# notes\n"
        );
    }

    // STUB: AC2
    #[test]
    fn scaffolds_agent_instructions_from_user_template() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        fs::write(
            paths.ward_agent_template(),
            "# {{display_name}}\nID={{ward_id}}\n",
        )
        .unwrap();

        let created = create_ward_from_template(&paths, "market-research").unwrap();
        assert_eq!(
            fs::read_to_string(created.path.join("AGENTS.md")).unwrap(),
            "# Market Research\nID=market-research\n"
        );
    }

    // STUB: AC3
    #[test]
    fn complete_bundle_rejects_invalid_doctrine_even_when_layout_omits_agents() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: notes, match: notes.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        fs::write(
            paths.ward_agent_template(),
            vec![b'x'; super::super::WARD_AGENT_TEMPLATE_MAX_BYTES + 1],
        )
        .unwrap();

        assert!(create_ward_from_template(&paths, "no-agent").is_err());
        assert!(!paths.ward_dir("no-agent").exists());
    }

    // STUB: AC3
    #[test]
    fn rendered_template_limit_failure_does_not_publish_partial_ward() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        fs::write(paths.ward_agent_template(), "{{ward_id}}".repeat(1000)).unwrap();

        let ward_id = "a".repeat(64);
        assert!(create_ward_from_template(&paths, &ward_id).is_err());
        assert!(!paths.ward_dir(&ward_id).exists());
    }

    // STUB: AC4
    #[test]
    fn existing_ward_and_doctrine_are_not_modified() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n  children:\n    - { id: agent-instructions, match: AGENTS.md, kind: file, format: markdown }\n",
        )
        .unwrap();
        fs::write(paths.ward_agent_template(), "# New doctrine\n").unwrap();
        let ward = paths.ward_dir("existing");
        fs::create_dir_all(&ward).unwrap();
        fs::write(ward.join("AGENTS.md"), "# User doctrine\n").unwrap();

        assert!(create_ward_from_template(&paths, "existing").is_err());
        assert_eq!(
            fs::read_to_string(ward.join("AGENTS.md")).unwrap(),
            "# User doctrine\n"
        );
    }

    #[test]
    fn ward_display_name_is_non_empty_for_separator_only_ids() {
        assert_eq!(
            ward_display_name("financial-analysis"),
            "Financial Analysis"
        );
        assert_eq!(ward_display_name("-"), "Ward (-)");
        assert_eq!(ward_display_name("___"), "Ward (___)");
    }

    #[test]
    fn refuses_existing_wards_and_unsafe_ids() {
        let vault = tempdir().unwrap();
        let paths = VaultPaths::new(vault.path().to_path_buf());
        paths.ensure_dirs_exist().unwrap();
        fs::create_dir_all(paths.templates_dir()).unwrap();
        fs::write(
            paths.ward_layout_template(),
            "apiVersion: zbot.dev/v1alpha1\nkind: WardLayout\nroot:\n  kind: directory\n",
        )
        .unwrap();
        create_ward_from_template(&paths, "safe").unwrap();
        assert!(create_ward_from_template(&paths, "safe").is_err());
        assert!(create_ward_from_template(&paths, "../escape").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tree_publication_is_confined_no_follow_and_no_clobber() {
        use std::os::unix::fs::symlink;

        let vault = tempdir().unwrap();
        let wards = vault.path().join("wards");
        let ward = wards.join("research");
        fs::create_dir_all(&ward).unwrap();
        let files = vec![
            (PathBuf::from("index.md"), Some(b"# Index\n".to_vec())),
            (PathBuf::from("nested"), None),
            (PathBuf::from("nested/note.md"), Some(b"# Note\n".to_vec())),
        ];

        publish_tree_no_replace(&wards, "research", &[], "alpha", &files).unwrap();
        assert_eq!(
            fs::read_to_string(ward.join("alpha/nested/note.md")).unwrap(),
            "# Note\n"
        );
        assert!(publish_tree_no_replace(&wards, "research", &[], "alpha", &files).is_err());
        assert!(publish_tree_no_replace(&wards, "research", &[], "ALPHA", &files).is_err());

        let outside = vault.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, ward.join("linked-parent")).unwrap();
        assert!(publish_tree_no_replace(
            &wards,
            "research",
            &["linked-parent".into()],
            "escape",
            &files,
        )
        .is_err());
        assert!(!outside.join("escape").exists());

        let unsafe_files = vec![(PathBuf::from("../escape.md"), Some(b"escape".to_vec()))];
        assert!(publish_tree_no_replace(&wards, "research", &[], "unsafe", &unsafe_files).is_err());
        assert!(!ward.join("escape.md").exists());
        assert!(!ward.join("unsafe").exists());
    }
}
