# Product version and release notes

Use one Semantic Versioning product version in Cargo.toml. Update Cargo.lock in
the same change; the About dialog and Windows executable metadata derive their
version from Cargo. The workspace JSON schema version is independent.

Following insamed-core's versioning convention: patch for fixes, minor for new
features or meaningful workflow improvements, and 1.0 only at production readiness.
One coherent release receives one version, rather than a version per commit.

Maintain CHANGELOG.md with Added/Changed/Fixed/Removed entries for engineering.
Maintain src/releases.rs with plain-language user-visible release notes rendered
in the icon menu. Describe delivered behavior, not planned capabilities.

Existing tags are immutable. Backfilled releases describe their actual commit;
they must not include later features. Build and verify before publishing new tags
or binaries. Release publishing follows the owner's authorization for this task.
