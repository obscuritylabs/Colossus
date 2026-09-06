use super::*;

/// Immutable-journal implementation of native integration connection state.
pub struct EventSourcedIntegrationRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedIntegrationRepository {
    /// Bind extension streams to the authoritative event journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(name: &str) -> String {
        format!("integration:{name}")
    }

    fn connection_events(
        &self,
        name: &str,
    ) -> Result<Vec<colossus_contracts::EventEnvelope>, StoreError> {
        self.journal.read_stream(&Self::stream(name))
    }

    fn reduce(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError> {
        let mut connection = None;
        for event in self.connection_events(name)? {
            match event.event_type.as_str() {
                "integration.connection_saved.v1" | "integration.disconnected.v1" => {
                    connection = Some(
                        serde_json::from_value(self.journal.decrypt_payload(&event)?)
                            .map_err(adapter)?,
                    );
                }
                _ => {}
            }
        }
        Ok(connection)
    }

    fn names(&self) -> Result<Vec<String>, StoreError> {
        self.stream_names("integration:")
    }

    fn stream_names(&self, prefix: &str) -> Result<Vec<String>, StoreError> {
        collect_stream_ids(self.journal.as_ref(), prefix)?
            .into_iter()
            .map(|stream_id| {
                stream_id
                    .strip_prefix(prefix)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        StoreError::Verification(format!(
                            "indexed stream {stream_id} does not match prefix {prefix}"
                        ))
                    })
            })
            .collect()
    }

    fn append(
        &self,
        connection: &IntegrationConnection,
        actor: Actor,
        event_type: &str,
    ) -> Result<(), StoreError> {
        let events = self.connection_events(&connection.name)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id: Self::stream(&connection.name),
            expected_stream_version: events.len() as u64,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("integration:{}", connection.name),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(connection).map_err(adapter)?,
        })?;
        Ok(())
    }
}

impl AggregateRepository for EventSourcedIntegrationRepository {
    fn get(&self, id: &str) -> Result<Option<Value>, StoreError> {
        self.get_integration(id)?
            .map(serde_json::to_value)
            .transpose()
            .map_err(adapter)
    }

    fn list(&self, limit: usize) -> Result<Vec<Value>, StoreError> {
        self.list_integrations(limit)?
            .into_iter()
            .map(serde_json::to_value)
            .collect::<Result<_, _>>()
            .map_err(adapter)
    }
}

impl IntegrationRepository for EventSourcedIntegrationRepository {
    fn get_integration(&self, name: &str) -> Result<Option<IntegrationConnection>, StoreError> {
        validate_name(name)?;
        self.reduce(name)
    }

    fn list_integrations(&self, limit: usize) -> Result<Vec<IntegrationConnection>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "integration list limit must be in 1..=1000".into(),
            ));
        }
        self.names()?
            .into_iter()
            .take(limit)
            .filter_map(|name| self.reduce(&name).transpose())
            .collect()
    }

    fn save_integration(
        &self,
        connection: IntegrationConnection,
        actor: Actor,
    ) -> Result<IntegrationConnection, StoreError> {
        validate_connection(&connection)?;
        if let Some(existing) = self.reduce(&connection.name)?
            && existing.connected_at != connection.connected_at
        {
            return Err(StoreError::Adapter(
                "integration connected_at is immutable".into(),
            ));
        }
        self.append(&connection, actor, "integration.connection_saved.v1")?;
        Ok(connection)
    }

    fn disconnect_integration(
        &self,
        name: &str,
        actor: Actor,
        updated_at: &str,
    ) -> Result<IntegrationConnection, StoreError> {
        let mut connection = self
            .reduce(name)?
            .ok_or_else(|| StoreError::NotFound(format!("integration {name}")))?;
        connection.status = IntegrationStatus::Disconnected;
        connection.updated_at = updated_at.into();
        validate_connection(&connection)?;
        self.append(&connection, actor, "integration.disconnected.v1")?;
        Ok(connection)
    }
}
