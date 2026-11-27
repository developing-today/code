//! Typed location values: R2, laptop and Drive locators, always
//! carried inside an explicit tenant/namespace scope (slice
//! A9115-locator-types, issue #10200; parent #9115; settled design:
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/
//! part-a-addressing/versioned-design.md`, section A-9115.2;
//! statements S3.21, S3.22, S3.26–S3.30).
//!
//! A location is a *typed* value, never an opaque path string
//! (A-9115.2): an R2 object key, a laptop filesystem path and a
//! Drive file do not resolve identically, so they are distinct
//! variants of [`TypedLocator`] with distinct field sets, and no
//! conversion exists between them. This slice owns only the locator
//! types and the scoped reference form; the location assertion
//! record, harvester and per-scope index are sibling slices, and
//! the crate-root/export wiring beyond the one `pub mod` line is
//! the wiring slice's (9110-02 / #10267).
//!
//! A locator is only meaningful inside the scope that observed it,
//! so the reference form, [`LocationRef`], can only be constructed
//! with a namespace and a tenant scope — there is no scope-less
//! location reference, and a blank tenant scope rejects with
//! [`LocationError::MissingTenantScope`]. The tenant scope is the
//! authenticated scope string of the tenancy contract (#9083, as
//! carried on the operation envelope in `port`); this module
//! consumes that string, it does not define tenancy.
//!
//! # Tagged text form
//!
//! Each locator has one canonical tagged text form, for the places
//! a locator must cross a text boundary (sidecars, indexes, logs):
//!
//! ```text
//! r2/<account>/<bucket>/<key>        key is the remainder, may contain '/'
//! laptop/<hostname><absolute path>   path begins with '/'
//! drive/<account>/<file-id>/<url>    url is the remainder
//! ```
//!
//! Parsing requires the explicit kind tag and never guesses: an
//! unknown tag is [`LocationError::UnsupportedLocatorKind`], and a
//! value parsed under a tag is always of that tag's kind — a laptop
//! path can never be read back as an R2 key, and no locator kind
//! coerces into another.

use std::fmt;
use std::str::FromStr;

use crate::namespace_definition::NamespaceId;

/// A location value failed validation or parsing. Each variant is a
/// distinct failure class so callers can classify without parsing
/// messages.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocationError {
    /// A required locator field was empty.
    #[error("location field {field} must not be empty")]
    EmptyField {
        /// The field that was empty (`account`, `bucket`, `key`,
        /// `hostname`, `path`, `file_id` or `url`).
        field: &'static str,
    },
    /// A locator field was present but not well formed.
    #[error("location field {field} is invalid: {problem}")]
    InvalidField {
        /// The field that was rejected.
        field: &'static str,
        /// What is wrong with it.
        problem: &'static str,
    },
    /// A location reference was constructed without a tenant scope.
    #[error("a location reference requires a tenant scope")]
    MissingTenantScope,
    /// The tagged text form named a locator kind this version does
    /// not know.
    #[error("unsupported locator kind tag: {tag}")]
    UnsupportedLocatorKind {
        /// The unrecognised tag.
        tag: String,
    },
    /// The tagged text form was missing required segments.
    #[error("malformed tagged locator: {problem}")]
    MalformedTaggedForm {
        /// What is missing.
        problem: &'static str,
    },
}

/// Which backend a [`TypedLocator`] addresses. The kind is intrinsic
/// to the value — it comes from the variant, never from inspecting
/// the field strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocatorKind {
    /// Cloudflare R2 object storage.
    R2,
    /// A file on a named laptop/host.
    Laptop,
    /// A Google Drive file.
    Drive,
}

impl LocatorKind {
    /// The tag naming this kind in the tagged text form.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::R2 => "r2",
            Self::Laptop => "laptop",
            Self::Drive => "drive",
        }
    }
}

impl fmt::Display for LocatorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.tag())
    }
}

/// A single path segment field (account, bucket, hostname,
/// file ID): non-empty, no `/`, no whitespace or control
/// characters, so the tagged text form stays unambiguous.
fn segment(field: &'static str, value: impl Into<String>) -> Result<String, LocationError> {
    let value = value.into();
    if value.is_empty() {
        return Err(LocationError::EmptyField { field });
    }
    if value.contains('/') || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(LocationError::InvalidField {
            field,
            problem: "must be a single segment without whitespace or control characters",
        });
    }
    Ok(value)
}

/// An R2 object key: non-empty, no control characters; `/` is
/// allowed (keys are hierarchical).
fn object_key(value: impl Into<String>) -> Result<String, LocationError> {
    let value = value.into();
    if value.is_empty() {
        return Err(LocationError::EmptyField { field: "key" });
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(LocationError::InvalidField {
            field: "key",
            problem: "must not contain control characters",
        });
    }
    Ok(value)
}

/// A laptop filesystem path: must be absolute, so a relative path
/// is never silently resolved against an unknown working
/// directory.
fn laptop_path(value: impl Into<String>) -> Result<String, LocationError> {
    let value = value.into();
    if value.is_empty() {
        return Err(LocationError::EmptyField { field: "path" });
    }
    if !value.starts_with('/') {
        return Err(LocationError::InvalidField {
            field: "path",
            problem: "must be an absolute path",
        });
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(LocationError::InvalidField {
            field: "path",
            problem: "must not contain control characters",
        });
    }
    Ok(value)
}

/// A Drive file URL: an `https` URL with a non-empty remainder, no
/// whitespace or control characters.
fn drive_url(value: impl Into<String>) -> Result<String, LocationError> {
    let value = value.into();
    if value.is_empty() {
        return Err(LocationError::EmptyField { field: "url" });
    }
    let well_formed = value
        .strip_prefix("https://")
        .is_some_and(|rest| !rest.is_empty())
        && !value.chars().any(|c| c.is_whitespace() || c.is_control());
    if !well_formed {
        return Err(LocationError::InvalidField {
            field: "url",
            problem: "must be an https URL without whitespace or control characters",
        });
    }
    Ok(value)
}

/// An object in Cloudflare R2: `{account, bucket, key}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct R2Locator {
    account: String,
    bucket: String,
    key: String,
}

impl R2Locator {
    /// A validated R2 locator.
    ///
    /// # Errors
    ///
    /// [`LocationError::EmptyField`] or
    /// [`LocationError::InvalidField`] naming the offending field.
    pub fn new(
        account: impl Into<String>,
        bucket: impl Into<String>,
        key: impl Into<String>,
    ) -> Result<Self, LocationError> {
        Ok(Self {
            account: segment("account", account)?,
            bucket: segment("bucket", bucket)?,
            key: object_key(key)?,
        })
    }

    /// The R2 account.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The bucket name.
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    /// The object key.
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// A file on a named host: `{hostname, path}`, the path absolute.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LaptopLocator {
    hostname: String,
    path: String,
}

impl LaptopLocator {
    /// A validated laptop locator.
    ///
    /// # Errors
    ///
    /// [`LocationError::EmptyField`] or
    /// [`LocationError::InvalidField`] naming the offending field;
    /// a relative `path` is `InvalidField`.
    pub fn new(
        hostname: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<Self, LocationError> {
        Ok(Self {
            hostname: segment("hostname", hostname)?,
            path: laptop_path(path)?,
        })
    }

    /// The host the file lives on.
    #[must_use]
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// The absolute path on that host.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// A file in Google Drive: `{account, file_id, url}`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DriveLocator {
    account: String,
    file_id: String,
    url: String,
}

impl DriveLocator {
    /// A validated Drive locator.
    ///
    /// # Errors
    ///
    /// [`LocationError::EmptyField`] or
    /// [`LocationError::InvalidField`] naming the offending field;
    /// a non-`https` `url` is `InvalidField`.
    pub fn new(
        account: impl Into<String>,
        file_id: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<Self, LocationError> {
        Ok(Self {
            account: segment("account", account)?,
            file_id: segment("file_id", file_id)?,
            url: drive_url(url)?,
        })
    }

    /// The Drive account holding the file.
    #[must_use]
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The Drive file ID.
    #[must_use]
    pub fn file_id(&self) -> &str {
        &self.file_id
    }

    /// The file's `https` URL.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
}

/// A typed location value. Exactly one variant per value; there is
/// deliberately no conversion between variants and no constructor
/// from an untagged string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypedLocator {
    /// An R2 object.
    R2(R2Locator),
    /// A file on a laptop/host.
    Laptop(LaptopLocator),
    /// A Drive file.
    Drive(DriveLocator),
}

impl TypedLocator {
    /// The kind of this locator, from its variant alone.
    #[must_use]
    pub const fn kind(&self) -> LocatorKind {
        match self {
            Self::R2(_) => LocatorKind::R2,
            Self::Laptop(_) => LocatorKind::Laptop,
            Self::Drive(_) => LocatorKind::Drive,
        }
    }

    /// The canonical tagged text form (see the module docs).
    #[must_use]
    pub fn to_tagged_string(&self) -> String {
        match self {
            Self::R2(r2) => format!("r2/{}/{}/{}", r2.account, r2.bucket, r2.key),
            Self::Laptop(laptop) => format!("laptop/{}{}", laptop.hostname, laptop.path),
            Self::Drive(drive) => {
                format!("drive/{}/{}/{}", drive.account, drive.file_id, drive.url)
            }
        }
    }

    /// Parse the canonical tagged text form. The kind tag is
    /// required and decisive: the result is always of the named
    /// kind, never a guess from the field shapes.
    ///
    /// # Errors
    ///
    /// [`LocationError::UnsupportedLocatorKind`] for an unknown
    /// tag, [`LocationError::MalformedTaggedForm`] for missing
    /// segments, and the field errors of the variant constructors.
    pub fn parse_tagged(text: &str) -> Result<Self, LocationError> {
        let (tag, rest) = text
            .split_once('/')
            .ok_or(LocationError::MalformedTaggedForm {
                problem: "missing kind tag",
            })?;
        if tag.is_empty() {
            return Err(LocationError::MalformedTaggedForm {
                problem: "missing kind tag",
            });
        }
        match tag {
            "r2" => {
                let mut parts = rest.splitn(3, '/');
                let account = next_part(&mut parts, "r2 locator needs account, bucket and key")?;
                let bucket = next_part(&mut parts, "r2 locator needs account, bucket and key")?;
                let key = next_part(&mut parts, "r2 locator needs account, bucket and key")?;
                Ok(Self::R2(R2Locator::new(account, bucket, key)?))
            }
            "laptop" => {
                let (hostname, tail) =
                    rest.split_once('/')
                        .ok_or(LocationError::MalformedTaggedForm {
                            problem: "laptop locator needs hostname and absolute path",
                        })?;
                Ok(Self::Laptop(LaptopLocator::new(
                    hostname,
                    format!("/{tail}"),
                )?))
            }
            "drive" => {
                let mut parts = rest.splitn(3, '/');
                let account =
                    next_part(&mut parts, "drive locator needs account, file id and url")?;
                let file_id =
                    next_part(&mut parts, "drive locator needs account, file id and url")?;
                let url = next_part(&mut parts, "drive locator needs account, file id and url")?;
                Ok(Self::Drive(DriveLocator::new(account, file_id, url)?))
            }
            other => Err(LocationError::UnsupportedLocatorKind {
                tag: other.to_string(),
            }),
        }
    }
}

/// The next required segment of a tagged form.
fn next_part<'a>(
    parts: &mut impl Iterator<Item = &'a str>,
    problem: &'static str,
) -> Result<&'a str, LocationError> {
    parts
        .next()
        .ok_or(LocationError::MalformedTaggedForm { problem })
}

impl fmt::Display for TypedLocator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_tagged_string())
    }
}

impl FromStr for TypedLocator {
    type Err = LocationError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse_tagged(text)
    }
}

impl From<R2Locator> for TypedLocator {
    fn from(locator: R2Locator) -> Self {
        Self::R2(locator)
    }
}

impl From<LaptopLocator> for TypedLocator {
    fn from(locator: LaptopLocator) -> Self {
        Self::Laptop(locator)
    }
}

impl From<DriveLocator> for TypedLocator {
    fn from(locator: DriveLocator) -> Self {
        Self::Drive(locator)
    }
}

/// A location reference: a [`TypedLocator`] bound to the namespace
/// and tenant scope it was observed in. The only constructor
/// requires both — a scope-less locator is not a reference and
/// cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocationRef {
    locator: TypedLocator,
    namespace: NamespaceId,
    tenant_scope: String,
}

impl LocationRef {
    /// A location reference in `namespace` under `tenant_scope`
    /// (the authenticated scope string of the tenancy contract,
    /// #9083).
    ///
    /// # Errors
    ///
    /// [`LocationError::MissingTenantScope`] when `tenant_scope` is
    /// empty or all whitespace;
    /// [`LocationError::InvalidField`] when it contains control
    /// characters.
    pub fn new(
        locator: TypedLocator,
        namespace: NamespaceId,
        tenant_scope: impl Into<String>,
    ) -> Result<Self, LocationError> {
        let tenant_scope = tenant_scope.into();
        if tenant_scope.trim().is_empty() {
            return Err(LocationError::MissingTenantScope);
        }
        if tenant_scope.chars().any(|c| c.is_control()) {
            return Err(LocationError::InvalidField {
                field: "tenant_scope",
                problem: "must not contain control characters",
            });
        }
        Ok(Self {
            locator,
            namespace,
            tenant_scope,
        })
    }

    /// The typed locator.
    #[must_use]
    pub fn locator(&self) -> &TypedLocator {
        &self.locator
    }

    /// The kind of the locator.
    #[must_use]
    pub fn kind(&self) -> LocatorKind {
        self.locator.kind()
    }

    /// The namespace the location was observed in.
    #[must_use]
    pub fn namespace(&self) -> &NamespaceId {
        &self.namespace
    }

    /// The tenant scope the location was observed under.
    #[must_use]
    pub fn tenant_scope(&self) -> &str {
        &self.tenant_scope
    }
}
