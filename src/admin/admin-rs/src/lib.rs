use reqwest::Client as HttpClient;
use url::Url;

use admin_http::StatusReport;
use admin_http::healthcheck::HealthcheckRes;
use admin_kernel::BoxedError;

#[derive(Clone)]
pub struct Client {
    http_client: HttpClient,
    base_url: Url,
}

impl Client {
    pub fn new(base_url: &str) -> Result<Self, BoxedError> {
        let http_client = HttpClient::new();
        let base_url = Url::parse(base_url)?;
        Ok(Self {
            http_client,
            base_url,
        })
    }

    pub async fn healthcheck(&self) -> Result<HealthcheckRes, BoxedError> {
        let url = self.base_url.join("/api/v1/__meta/healthcheck")?;
        let response = self.http_client.get(url).send().await?;
        let healthcheck: HealthcheckRes = response.json().await?;
        Ok(healthcheck)
    }

    /// One `aka status` snapshot: services, proxyd health and the routes.
    /// No credentials are sent, because the API asks for none.
    pub async fn status(&self) -> Result<StatusReport, BoxedError> {
        let url = self.base_url.join("/api/v1/status")?;
        let response = self.http_client.get(url).send().await?;
        let report: StatusReport = response.json().await?;
        Ok(report)
    }
}
