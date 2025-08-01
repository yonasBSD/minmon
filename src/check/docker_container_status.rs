use super::DataSource;
use crate::{config, measurement};
use crate::{Error, PlaceholderMap, Result};
use async_trait::async_trait;
use measurement::Measurement;

pub struct DockerContainerStatus {
    socket_path: String,
    containers: Vec<String>,
}

enum ContainerStatus {
    NotFound,
    Found(bollard::models::ContainerState),
}

impl DockerContainerStatus {
    async fn container_state(docker: &bollard::Docker, container: &str) -> Result<ContainerStatus> {
        let response = docker
            .inspect_container(
                container,
                Some(bollard::query_parameters::InspectContainerOptions { size: false }),
            )
            .await;
        if let Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            message: _,
        }) = response
        {
            return Ok(ContainerStatus::NotFound);
        }
        response
            .map_err(|x| Error(format!("Docker error: {x}")))
            .and_then(|x| {
                x.state
                    .ok_or_else(|| Error(String::from("Could not read container state.")))
            })
            .map(ContainerStatus::Found)
    }

    async fn container_running_and_healthy(
        docker: &bollard::Docker,
        container: &str,
    ) -> Result<bool> {
        let state = match Self::container_state(docker, container).await? {
            ContainerStatus::NotFound => return Ok(false),
            ContainerStatus::Found(state) => state,
        };
        let status = state
            .status
            .ok_or_else(|| Error(String::from("Could not read container status.")))
            .map(|x| x == bollard::models::ContainerStateStatusEnum::RUNNING)?;
        let health = state
            .health
            .and_then(|health| health.status)
            .map(|x| {
                x == bollard::models::HealthStatusEnum::HEALTHY
                    || x == bollard::models::HealthStatusEnum::NONE
            })
            .unwrap_or(true);
        Ok(status && health)
    }
}

impl TryFrom<&config::Check> for DockerContainerStatus {
    type Error = Error;

    fn try_from(check: &config::Check) -> std::result::Result<Self, Self::Error> {
        if let config::CheckType::DockerContainerStatus(container_status) = &check.type_ {
            Ok(Self {
                socket_path: container_status.socket_path.clone(),
                containers: container_status.containers.clone(),
            })
        } else {
            panic!();
        }
    }
}

#[async_trait]
impl DataSource for DockerContainerStatus {
    type Item = measurement::BinaryState;

    async fn get_data(
        &mut self,
        _placeholders: &mut PlaceholderMap,
    ) -> Result<Vec<Result<Option<Self::Item>>>> {
        let docker = bollard::Docker::connect_with_unix(
            &self.socket_path,
            u64::MAX,
            bollard::API_DEFAULT_VERSION,
        )
        .map_err(|x| Error(format!("Could not create docker client: {x}")))?;
        let mut res = Vec::new();
        for container in self.containers.iter() {
            res.push(
                Self::container_running_and_healthy(&docker, container)
                    .await
                    .and_then(Self::Item::new)
                    .map(Some),
            );
        }
        Ok(res)
    }

    fn format_data(&self, data: &Self::Item) -> String {
        match data.data() {
            true => "container running (and healthy)",
            false => "container not running (or unhealthy)",
        }
        .into()
    }

    fn ids(&self) -> &[String] {
        &self.containers[..]
    }
}
