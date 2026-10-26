//! Match the process identity to installer-owned shortcuts before creating UI.

pub(super) fn configure(identifier: &str) -> bool {
    winsafe::SetCurrentProcessExplicitAppUserModelID(identifier).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn explicit_identity_is_set_on_the_current_test_process() {
        let identifier = "com.panitacraft.client.identity-test";
        assert!(super::configure(identifier));
        assert_eq!(
            winsafe::GetCurrentProcessExplicitAppUserModelID()
                .unwrap_or_else(|_| panic!("current process identity")),
            identifier
        );
    }
}
