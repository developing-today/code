    use super::*;
    use crate::directory::{Actor, Level, Member, Refusal};

    struct Staff {
        core: WorldCore,
        member: String,
        writer: String,
        manager: String,
        moderator: String,
        admin: String,
        stranger: String,
    }

    fn account(core: &mut WorldCore, name: &str) -> String {
        let (credential, _) = core.directory_mut().sign_up(name).unwrap();
        core.directory().account_for(&credential).unwrap()
    }

    fn seat(core: &mut WorldCore, group: u64, name: &str, level: Level) -> String {
        let id = account(core, name);
        core.directory_mut()
            .set_member(
                Actor::Server,
                group,
                Member::Account { id: id.clone() },
                Some(level),
            )
            .unwrap();
        id
    }

    /// One group with a holder of each level, plus an account outside it.
    fn staffed() -> Staff {
        let mut core = WorldCore::new("staffed", WorldLimits::default()).unwrap();
        let founder = account(&mut core, "Founder");
        let (group, _) = core
            .directory_mut()
            .create_group(&founder, "staff", "", WorldScopes::GUEST)
            .unwrap();
        Staff {
            member: seat(&mut core, group, "Member", Level::Access),
            writer: seat(&mut core, group, "Writer", Level::Write),
            manager: seat(&mut core, group, "Manager", Level::Manage),
            moderator: seat(&mut core, group, "Moderator", Level::Moderator),
            admin: seat(&mut core, group, "Admin", Level::Admin),
            stranger: account(&mut core, "Stranger"),
            core,
        }
    }

    #[tokio::test]
    async fn worlds_start_isolated() {
        assert_eq!(Isolation::default(), Isolation::Isolated);
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        assert_eq!(handle.isolation().await.unwrap(), Isolation::Isolated);
    }

    #[tokio::test]
    async fn only_a_moderator_or_an_admin_sets_isolation() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_isolation(Authority::Signed(who.clone()), Isolation::Unisolated)
                .await
                .unwrap_err();
            assert!(
                matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_))),
                "{who} must not change isolation"
            );
        }
        assert_eq!(handle.isolation().await.unwrap(), Isolation::Isolated);

        assert_eq!(
            handle
                .set_isolation(
                    Authority::Signed(staff.moderator.clone()),
                    Isolation::Unisolated
                )
                .await
                .unwrap(),
            Isolation::Unisolated
        );
        assert_eq!(
            handle
                .set_isolation(Authority::Signed(staff.admin.clone()), Isolation::Isolated)
                .await
                .unwrap(),
            Isolation::Isolated
        );
        assert_eq!(
            handle
                .set_isolation(Authority::Admin, Isolation::Unisolated)
                .await
                .unwrap(),
            Isolation::Unisolated
        );
    }

    #[test]
    fn a_checkpoint_keeps_the_isolation() {
        let mut core = WorldCore::new("iso", WorldLimits::default()).unwrap();
        core.isolation = Isolation::Unisolated;
        core.active_module_hash = Some("h".to_owned());
        let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
        assert!(entries.iter().any(|entry| matches!(
            entry,
            JournalEntry::Isolation {
                isolation: Isolation::Unisolated
            }
        )));
    }

    fn forbidden(error: &anyhow::Error) -> bool {
        matches!(error.downcast_ref::<Refusal>(), Some(Refusal::Forbidden(_)))
    }

    #[tokio::test]
    async fn only_a_moderator_or_an_admin_sets_cross_world_write() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_cross_world_write(Authority::Signed(who.clone()), true)
                .await
                .unwrap_err();
            assert!(
                forbidden(&error),
                "{who} must not change cross-world writes"
            );
        }
        assert!(!handle.policy().await.unwrap().cross_world_write);

        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), true)
            .await
            .unwrap();
        assert!(policy.cross_world_write && !policy.cross_world_forced);
        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write);
    }

    #[tokio::test]
    async fn only_the_admin_forces_cross_world_writes() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        for who in [&staff.moderator, &staff.admin] {
            let error = handle
                .force_cross_world_write(Authority::Signed(who.clone()))
                .await
                .unwrap_err();
            assert!(forbidden(&error), "{who} must not force cross-world writes");
        }
        assert!(!handle.policy().await.unwrap().cross_world_forced);
    }

    #[tokio::test]
    async fn an_isolated_world_cannot_be_forced() {
        let handle = WorldHandle::spawn(WorldCore::new("w", WorldLimits::default()).unwrap());
        let error = handle
            .force_cross_world_write(Authority::Admin)
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        assert!(error.to_string().contains("isolated"), "{error}");
        assert!(!handle.policy().await.unwrap().cross_world_forced);
    }

    #[tokio::test]
    async fn a_forced_write_stays_on_against_a_moderator() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_isolation(Authority::Admin, Isolation::Unisolated)
            .await
            .unwrap();
        handle
            .force_cross_world_write(Authority::Admin)
            .await
            .unwrap();

        let error = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        let policy = handle.policy().await.unwrap();
        assert!(policy.cross_world_write && policy.cross_world_forced);

        let policy = handle
            .set_cross_world_write(Authority::Admin, false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write && !policy.cross_world_forced);
    }

    #[tokio::test]
    async fn a_moderator_turns_off_a_write_that_was_not_forced() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), true)
            .await
            .unwrap();
        let policy = handle
            .set_cross_world_write(Authority::Signed(staff.moderator.clone()), false)
            .await
            .unwrap();
        assert!(!policy.cross_world_write);
    }

    #[tokio::test]
    async fn file_scope_is_moderated_and_the_override_is_admin_only() {
        let staff = staffed();
        let handle = WorldHandle::spawn(staff.core);
        assert_eq!(
            handle.policy().await.unwrap(),
            WorldPolicy {
                isolation: Isolation::Isolated,
                cross_world_write: false,
                cross_world_forced: false,
                file_scope: FileScope::Confined,
                file_write_override: false,
            }
        );
        for who in [
            &staff.member,
            &staff.writer,
            &staff.manager,
            &staff.stranger,
        ] {
            let error = handle
                .set_file_scope(
                    Authority::Signed(who.clone()),
                    FileScope::Unrestricted,
                    None,
                )
                .await
                .unwrap_err();
            assert!(forbidden(&error), "{who} must not change the file scope");
        }

        let error = handle
            .set_file_scope(
                Authority::Signed(staff.moderator.clone()),
                FileScope::Unrestricted,
                Some(true),
            )
            .await
            .unwrap_err();
        assert!(forbidden(&error));
        assert_eq!(
            handle.policy().await.unwrap().file_scope,
            FileScope::Confined
        );

        let policy = handle
            .set_file_scope(
                Authority::Signed(staff.moderator.clone()),
                FileScope::Unrestricted,
                None,
            )
            .await
            .unwrap();
        assert_eq!(policy.file_scope, FileScope::Unrestricted);
        assert!(!policy.file_write_override);

        let policy = handle
            .set_file_scope(Authority::Admin, FileScope::Unrestricted, Some(true))
            .await
            .unwrap();
        assert!(policy.file_write_override);
    }

    #[test]
    fn file_access_follows_isolation_scope_and_override() {
        use FileMode::{Read, Write};
        use FileScope::{Confined, Unrestricted};
        use Isolation::{Isolated, Unisolated};
        let policy = |isolation, file_scope, file_write_override| WorldPolicy {
            isolation,
            cross_world_write: false,
            cross_world_forced: false,
            file_scope,
            file_write_override,
        };
        // (policy, mode, inside own root, allowed)
        let cases = [
            (policy(Unisolated, Confined, false), Read, true, true),
            (policy(Unisolated, Confined, false), Read, false, false),
            (policy(Unisolated, Confined, false), Write, true, true),
            (policy(Unisolated, Confined, false), Write, false, false),
            (policy(Unisolated, Unrestricted, false), Read, false, true),
            (policy(Unisolated, Unrestricted, false), Write, false, true),
            (policy(Isolated, Confined, false), Read, true, true),
            (policy(Isolated, Confined, false), Read, false, false),
            (policy(Isolated, Confined, false), Write, true, false),
            (policy(Isolated, Unrestricted, false), Read, false, true),
            (policy(Isolated, Unrestricted, false), Write, false, false),
            (policy(Isolated, Unrestricted, true), Write, false, true),
            (policy(Isolated, Confined, true), Write, true, true),
            (policy(Isolated, Confined, true), Write, false, false),
        ];
        for (policy, mode, inside_root, expected) in cases {
            assert_eq!(
                policy.file_allowed(mode, inside_root),
                expected,
                "{policy:?} {mode:?} inside root {inside_root}"
            );
        }
    }

    #[test]
    fn a_default_world_reads_only_inside_its_root() {
        let policy = WorldCore::new("w", WorldLimits::default())
            .unwrap()
            .policy();
        assert_eq!(policy.file_scope, FileScope::Confined);
        assert!(!policy.file_write_override);
        assert!(policy.file_allowed(FileMode::Read, true));
        assert!(!policy.file_allowed(FileMode::Read, false));
        assert!(!policy.file_allowed(FileMode::Write, true));
    }

    #[test]
    fn a_checkpoint_keeps_the_sharing_settings() {
        for (cross_world_write, cross_world_forced) in [(true, false), (true, true), (false, false)]
        {
            let mut core = WorldCore::new("share", WorldLimits::default()).unwrap();
            core.isolation = Isolation::Unisolated;
            core.cross_world_write = cross_world_write;
            core.cross_world_forced = cross_world_forced;
            core.file_scope = FileScope::Unrestricted;
            core.file_write_override = true;
            core.active_module_hash = Some("h".to_owned());
            let entries = core.checkpoint_entries("snapshot".to_owned()).unwrap();
            let restored = WorldCore::restore("share", WorldLimits::default(), entries).unwrap();
            assert_eq!(restored.core.policy(), core.policy());
        }
    }
}
