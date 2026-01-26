use anyhow::{Error, Result};
use tokio_rusqlite::Connection;

use super::models::PushSubscription;

pub async fn find_all_notification_subscriptions(
    db: &Connection,
) -> Result<Vec<PushSubscription>, Error> {
    let subscriptions = db.call(|conn| {
        let mut stmt = conn.prepare("SELECT endpoint, p256dh, auth FROM push_subscription")?;
        let rows = stmt
            .query_map([], |i| {
                Ok(PushSubscription {
                    endpoint: i.get(0)?,
                    p256dh: i.get(1)?,
                    auth: i.get(2)?,
                })
            })?
            .filter_map(Result::ok)
            .collect::<Vec<PushSubscription>>();
        Ok(rows)
    });
    Ok(subscriptions.await?)
}
