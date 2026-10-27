    use super::*;
    use crate::directory::{Actor, Tier};

    fn core() -> WorldCore {
        WorldCore::new("w", WorldLimits::default()).unwrap()
    }

    fn account(core: &mut WorldCore, name: &str) -> (String, String) {
        let (credential, _) = core.directory_mut().sign_up(name).unwrap();
        let id = core.directory().account_for(&credential).unwrap();
        (id, credential)
    }

    #[tokio::test]
    async fn a_friend_delegation_through_the_handle_is_held_to_the_friendship() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (cy, _) = account(&mut core, "Cy");
        core.directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, ann_token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let handle = WorldHandle::spawn(core);

        let (_, bo_token) = handle
            .delegate_to_friend(
                ann_token.clone(),
                bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo",
            )
            .await
            .unwrap();
        handle.chat(bo_token, "hi").await.unwrap();
        assert!(
            handle
                .delegate_to_friend(
                    ann_token,
                    cy,
                    WorldScopes::GUEST,
                    CapabilityBounds::default(),
                    "Cy"
                )
                .await
                .is_err(),
            "only friends may be delegated to"
        );
    }

    #[tokio::test]
    async fn a_bounded_issue_stops_after_its_uses() {
        let handle = WorldHandle::spawn(core());
        let bounds = CapabilityBounds {
            expires_at: None,
            uses: Some(1),
        };
        let (_, token) = handle
            .issue_bounded("Ann", WorldScopes::GUEST, bounds)
            .await
            .unwrap();
        handle.chat(token.clone(), "once").await.unwrap();
        assert!(handle.chat(token, "twice").await.is_err());
    }

    fn befriend(core: &mut WorldCore, a: &str, a_credential: &str, b: &str, b_credential: &str) {
        let world = core.world_id().to_owned();
        core.directory_mut()
            .request_friend(a_credential, b, &world, 1)
            .unwrap();
        core.directory_mut()
            .accept_friend(b_credential, a, &world, 2)
            .unwrap();
    }

    #[test]
    fn a_ceiling_that_allows_chat_allows_reading_it() {
        let mut core = core();
        let (_, ann_credential) = account(&mut core, "Ann");
        let (_, guest) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let (_, reader) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::JOIN.union(WorldScopes::HISTORY),
                CapabilityBounds::default(),
                "Ann reads",
            )
            .unwrap();
        core.chat(&guest, "hello").unwrap();
        assert_eq!(core.snapshot(&reader).unwrap().events.len(), 1);
        assert!(core.chat(&reader, "hi").is_err());
    }

    #[test]
    fn an_account_capability_is_held_to_its_ceiling() {
        let mut core = core();
        let (_, ann_credential) = account(&mut core, "Ann");
        assert!(
            core.issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann"
            )
            .is_err(),
            "a registered account cannot be issued delegate scope"
        );
        assert!(
            core.issue_for_account(
                "acct.forged.00",
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann"
            )
            .is_err()
        );
        let (_, token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        core.chat(&token, "hello").unwrap();
        core.directory_mut()
            .set_tier(Actor::Server, Tier::Registered, WorldScopes::JOIN)
            .unwrap();
        assert!(
            core.chat(&token, "again").is_err(),
            "a narrowed tier narrows the capability"
        );
        assert!(core.snapshot(&token).is_ok());
    }

    #[test]
    fn an_account_capability_serializes_its_subject_but_no_secret() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (participant, _) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();
        let entry = core.issued_entry(participant.id).unwrap();
        let text = serde_json::to_string(&entry).unwrap();
        assert!(text.contains(&ann));
        let secret = ann_credential.rsplit('.').next().unwrap();
        assert!(!text.contains(secret));
        let back: JournalEntry = serde_json::from_str(&text).unwrap();
        assert_eq!(back, entry);
    }

    #[test]
    fn a_delegation_to_a_friend_is_held_to_the_friends_ceiling_and_ends_with_the_friendship() {
        let mut core = core();
        let (ann, ann_credential) = account(&mut core, "Ann");
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (cy, _) = account(&mut core, "Cy");
        core.directory_mut()
            .verify_account(Actor::Server, &ann)
            .unwrap();
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, ann_token) = core
            .issue_for_account(
                &ann_credential,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Ann",
            )
            .unwrap();

        assert!(
            core.delegate_to_friend(
                &ann_token,
                &bo,
                WorldScopes::ALL,
                CapabilityBounds::default(),
                "Bo"
            )
            .is_err(),
            "bo's ceiling does not include delegate"
        );
        assert!(
            core.delegate_to_friend(
                &ann_token,
                &cy,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Cy"
            )
            .is_err(),
            "only friends may be delegated to"
        );
        let (_, bo_token) = core
            .delegate_to_friend(
                &ann_token,
                &bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo",
            )
            .unwrap();
        core.chat(&bo_token, "hi").unwrap();
        assert!(
            core.attenuate(
                &bo_token,
                WorldScopes::JOIN,
                CapabilityBounds::default(),
                "Di"
            )
            .is_err(),
            "bo's delegation carries no delegate scope"
        );

        core.directory_mut()
            .remove_friend(&ann_credential, &bo)
            .unwrap();
        assert!(
            core.snapshot(&bo_token).is_err(),
            "the delegation ends with the friendship"
        );
        assert!(core.snapshot(&ann_token).is_ok());
    }

    #[test]
    fn only_an_account_capability_may_delegate_to_a_friend() {
        let mut core = core();
        let (bo, bo_credential) = account(&mut core, "Bo");
        let (ann, ann_credential) = account(&mut core, "Ann");
        befriend(&mut core, &ann, &ann_credential, &bo, &bo_credential);
        let (_, anonymous) = core
            .issue_capability("Guest", WorldScopes::ALL, CapabilityBounds::default())
            .unwrap();
        assert!(
            core.delegate_to_friend(
                &anonymous,
                &bo,
                WorldScopes::GUEST,
                CapabilityBounds::default(),
                "Bo"
            )
            .is_err()
        );
    }
}
