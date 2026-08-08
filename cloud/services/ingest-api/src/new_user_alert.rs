use reqwest::Client;
use serde_json::json;
use work_insights_db::identity::AppIdentity;

use crate::NewUserAlertConfig;

pub(crate) fn schedule(client: Client, config: Option<NewUserAlertConfig>, identity: AppIdentity) {
    let Some(config) = config else {
        return;
    };

    tokio::spawn(async move {
        let organization = identity
            .org_name
            .or_else(|| identity.org.and_then(|organization| organization.name))
            .unwrap_or_else(|| "Unknown organization".to_string());
        let display_name = identity.display_name.unwrap_or_else(|| "—".to_string());
        let text = format!(
            "New Dystil user\n\nName: {display_name}\nEmail: {}\nOrganization: {organization}",
            identity.email
        );

        match client
            .post(config.webhook_url)
            .json(&json!({ "text": text }))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                tracing::info!(
                    user_id = %identity.user_id,
                    "new-user Google Chat alert delivered"
                );
            }
            Ok(response) => {
                tracing::warn!(
                    user_id = %identity.user_id,
                    status = %response.status(),
                    "new-user Google Chat alert failed; notification dropped"
                );
            }
            Err(error) => {
                tracing::warn!(
                    user_id = %identity.user_id,
                    %error,
                    "new-user Google Chat alert failed; notification dropped"
                );
            }
        }
    });
}
