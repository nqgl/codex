use crate::GroupMailStore;
use crate::store::MemberStatus;
use crate::store::PendingMessage;
use crate::store::Priority;
use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn membership_survives_reopen_and_offline_mail_waits() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let alice = ThreadId::new();
    let bob = ThreadId::new();
    let store = GroupMailStore::open(home.path()).await?;
    store.join(alice, "research", "alice").await?;
    store.join(bob, "research", "bob").await?;
    store.mark_online(alice, "alice-run").await?;
    store.mark_online(bob, "bob-run").await?;
    store.mark_offline(bob, "bob-run").await?;

    let receipt = store
        .send(alice, Some(&["bob".to_string()]), "hello", Priority::High)
        .await?;
    assert_eq!(
        receipt.recipients,
        vec![MemberStatus {
            name: "bob".to_string(),
            online: false,
        }]
    );
    drop(store);

    let reopened = GroupMailStore::open(home.path()).await?;
    assert_eq!(reopened.pending(bob, "bob-run").await?, Vec::new());
    reopened.mark_online(bob, "bob-resumed").await?;
    let pending = reopened.pending(bob, "bob-resumed").await?;
    assert_eq!(
        pending,
        vec![PendingMessage {
            id: pending[0].id,
            sender: "alice".to_string(),
            body: "hello".to_string(),
            high: true,
        }]
    );
    reopened
        .acknowledge(bob, "bob-resumed", &[pending[0].id])
        .await?;
    assert_eq!(reopened.pending(bob, "bob-resumed").await?, Vec::new());
    Ok(())
}

#[tokio::test]
async fn multi_recipient_send_is_atomic_and_preserves_order() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let store = GroupMailStore::open(home.path()).await?;
    let alice = ThreadId::new();
    let bob = ThreadId::new();
    let carol = ThreadId::new();
    store.join(alice, "team", "alice").await?;
    store.join(bob, "team", "bob").await?;
    store.join(carol, "team", "carol").await?;
    store.mark_online(bob, "bob-run").await?;
    store.mark_online(carol, "carol-run").await?;

    store
        .send(alice, Some(&["bob".into()]), "first", Priority::Low)
        .await?;
    store
        .send(
            alice,
            Some(&["bob".into(), "carol".into()]),
            "second",
            Priority::High,
        )
        .await?;
    let bob_mail = store.pending(bob, "bob-run").await?;
    let carol_mail = store.pending(carol, "carol-run").await?;
    assert_eq!(
        bob_mail
            .iter()
            .map(|mail| (mail.body.as_str(), mail.high))
            .collect::<Vec<_>>(),
        vec![("first", false), ("second", true)]
    );
    assert_eq!(
        carol_mail
            .iter()
            .map(|mail| (mail.body.as_str(), mail.high))
            .collect::<Vec<_>>(),
        vec![("second", true)]
    );

    assert!(
        store
            .send(
                alice,
                Some(&["bob".into(), "missing".into()]),
                "not sent",
                Priority::High
            )
            .await
            .is_err()
    );
    assert_eq!(store.pending(bob, "bob-run").await?, bob_mail);
    assert_eq!(store.pending(carol, "carol-run").await?, carol_mail);
    Ok(())
}

#[tokio::test]
async fn oversized_mail_fails_without_partial_delivery() -> anyhow::Result<()> {
    let home = tempfile::tempdir()?;
    let store = GroupMailStore::open(home.path()).await?;
    let alice = ThreadId::new();
    let bob = ThreadId::new();
    store.join(alice, "team", "alice").await?;
    store.join(bob, "team", "bob").await?;
    store.mark_online(bob, "bob-run").await?;
    let too_large = "x".repeat(4_001);
    assert!(
        store
            .send(alice, Some(&["bob".into()]), &too_large, Priority::High)
            .await
            .is_err()
    );
    assert_eq!(store.pending(bob, "bob-run").await?, Vec::new());

    let accepted = "y".repeat(4_000);
    store
        .send(alice, Some(&["bob".into()]), &accepted, Priority::Low)
        .await?;
    store
        .send(alice, Some(&["bob".into()]), &accepted, Priority::High)
        .await?;
    assert!(
        store
            .send(alice, Some(&["bob".into()]), "one more", Priority::High)
            .await
            .is_err()
    );
    let pending = store.pending(bob, "bob-run").await?;
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].body, accepted);
    assert_eq!(pending[1].body, accepted);
    Ok(())
}
