//! Settings for [`crate::spa_router`] and the dev-server URL helper.

/// Configuration for [`spa_router`].
#[derive(Debug, Clone)]
pub struct SpaConfig<E> {
    /// Embedded `dist` bundle type: a unit struct, used in release builds.
    pub bundle: E,
    /// Env var holding the dev-server base URL (debug builds only).
    pub dev_url_env: String,
    /// Default dev-server base URL when the env var is unset (debug only).
    pub dev_url_default: String,
    /// Path prefixes reserved for the caller's APIs; never served or proxied.
    pub api_prefixes: Vec<String>,
}

impl<E: Default> Default for SpaConfig<E> {
    fn default() -> Self {
        Self {
            bundle: E::default(),
            dev_url_env: "WEB_DEV_URL".to_string(),
            dev_url_default: "http://127.0.0.1:5173".to_string(),
            api_prefixes: vec!["/api/".to_string()],
        }
    }
}

impl<E> SpaConfig<E> {
    /// Build a config explicitly. `api_prefixes` may hold any number of
    /// prefixes, for example `["/api/", "/auth/"]`.
    pub fn new<I, P>(
        bundle: E,
        dev_url_env: impl Into<String>,
        dev_url_default: impl Into<String>,
        api_prefixes: I,
    ) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        Self {
            bundle,
            dev_url_env: dev_url_env.into(),
            dev_url_default: dev_url_default.into(),
            api_prefixes: api_prefixes.into_iter().map(Into::into).collect(),
        }
    }
}

/// Resolve the dev-server base URL: `$env_var`, or `default` when unset.
/// A trailing `/` is trimmed so path joins never duplicate a slash.
pub fn dev_base(env_var: &str, default: &str) -> String {
    let base = std::env::var(env_var).unwrap_or_else(|_| default.to_string());
    base.trim_end_matches('/').to_string()
}
