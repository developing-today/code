//! HOME resolution shared by the queue client and broker processes.

pub(crate) const HOME_REQUIRED: &str = "HOME is unset or empty";

pub(crate) fn resolve(value: Option<&str>) -> Result<&str, &'static str> {
    value.filter(|home| !home.is_empty()).ok_or(HOME_REQUIRED)
}

pub(crate) fn from_env() -> Result<String, &'static str> {
    let value = std::env::var("HOME").ok();
    resolve(value.as_deref()).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_and_empty_home_fail_with_the_same_named_error() {
        assert_eq!(resolve(None), Err(HOME_REQUIRED));
        assert_eq!(resolve(Some("")), Err(HOME_REQUIRED));
    }

    #[test]
    fn set_home_is_preserved_exactly() {
        assert_eq!(resolve(Some("/home/test")), Ok("/home/test"));
        assert_eq!(resolve(Some(" /home/test ")), Ok(" /home/test "));
    }
}
