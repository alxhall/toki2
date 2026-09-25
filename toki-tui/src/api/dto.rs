use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDto {
    pub project_id: String,
    pub project_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDto {
    pub activity: String,
    pub activity_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartTimerRequest {
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub activity_id: Option<String>,
    pub activity_name: Option<String>,
    pub user_note: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveTimerRequest {
    pub user_note: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::SaveTimerRequest;

    #[test]
    fn save_timer_payload_matches_published_contract() {
        let request = SaveTimerRequest {
            user_note: Some("Work note".to_string()),
        };

        assert_eq!(
            serde_json::to_value(request).unwrap(),
            serde_json::json!({ "userNote": "Work note" })
        );
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateActiveTimerRequest {
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub activity_id: Option<String>,
    pub activity_name: Option<String>,
    pub user_note: Option<String>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub start_time: Option<time::OffsetDateTime>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditEntryRequest<'a> {
    pub project_registration_id: &'a str,
    pub project_id: &'a str,
    pub project_name: &'a str,
    pub activity_id: &'a str,
    pub activity_name: &'a str,
    pub start_time: String,
    pub end_time: String,
    pub user_note: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteEntryRequest<'a> {
    pub project_registration_id: &'a str,
}
