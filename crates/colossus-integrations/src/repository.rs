use super::*;

/// Immutable-journal implementation of extension connection state.
pub struct EventSourcedExtensionRepository {
    journal: Arc<dyn EventJournal>,
}

impl EventSourcedExtensionRepository {
    /// Bind extension streams to the authoritative event journal.
    pub fn new(journal: Arc<dyn EventJournal>) -> Self {
        Self { journal }
    }

    fn stream(name: &str) -> String {
        format!("integration:{name}")
    }

    fn pack_stream(name: &str) -> String {
        format!("pack:{name}")
    }

    fn trust_stream(publisher: &str, key_id: &str) -> String {
        format!("publisher-trust:{publisher}:{key_id}")
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

    fn reduce_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError> {
        let mut installation = None;
        for event in self.journal.read_stream(&Self::pack_stream(name))? {
            if matches!(
                event.event_type.as_str(),
                "pack.installed.v1"
                    | "pack.enabled.v1"
                    | "pack.disabled.v1"
                    | "pack.uninstalled.v1"
            ) {
                installation = Some(
                    serde_json::from_value(self.journal.decrypt_payload(&event)?)
                        .map_err(adapter)?,
                );
            }
        }
        Ok(installation)
    }

    fn reduce_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError> {
        let events = self
            .journal
            .read_stream(&Self::trust_stream(publisher, key_id))?;
        events
            .last()
            .map(|event| {
                serde_json::from_value(self.journal.decrypt_payload(event)?).map_err(adapter)
            })
            .transpose()
    }

    fn append_pack(
        &self,
        installation: &PackInstallation,
        actor: Actor,
        event_type: &str,
    ) -> Result<(), StoreError> {
        let stream_id = Self::pack_stream(&installation.manifest.name);
        let events = self.journal.read_stream(&stream_id)?;
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: events.len() as u64,
            classification: EventClassification::Domain,
            event_type: event_type.into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("pack:{}", installation.manifest.name),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(installation).map_err(adapter)?,
        })?;
        Ok(())
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

impl AggregateRepository for EventSourcedExtensionRepository {
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

impl ExtensionRepository for EventSourcedExtensionRepository {
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

    fn get_pack(&self, name: &str) -> Result<Option<PackInstallation>, StoreError> {
        validate_name(name)?;
        self.reduce_pack(name)
    }

    fn list_packs(&self, limit: usize) -> Result<Vec<PackInstallation>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "pack list limit must be in 1..=1000".into(),
            ));
        }
        self.stream_names("pack:")?
            .into_iter()
            .take(limit)
            .filter_map(|name| self.reduce_pack(&name).transpose())
            .collect()
    }

    fn install_pack(
        &self,
        installation: PackInstallation,
        actor: Actor,
    ) -> Result<PackInstallation, StoreError> {
        validate_name(&installation.manifest.name)?;
        if installation.status == PackStatus::Uninstalled {
            return Err(StoreError::Adapter(
                "a new pack installation cannot start uninstalled".into(),
            ));
        }
        if let Some(existing) = self.reduce_pack(&installation.manifest.name)?
            && existing.status != PackStatus::Uninstalled
        {
            return Err(StoreError::Adapter(format!(
                "pack {} is already installed",
                installation.manifest.name
            )));
        }
        self.append_pack(&installation, actor, "pack.installed.v1")?;
        Ok(installation)
    }

    fn install_packs(
        &self,
        installations: Vec<PackInstallation>,
        actor: Actor,
    ) -> Result<Vec<PackInstallation>, StoreError> {
        if installations.is_empty() {
            return Err(StoreError::Adapter(
                "pack installation batch cannot be empty".into(),
            ));
        }
        let mut names = BTreeSet::new();
        let mut events = Vec::with_capacity(installations.len());
        for installation in &installations {
            validate_name(&installation.manifest.name)?;
            if installation.status == PackStatus::Uninstalled {
                return Err(StoreError::Adapter(
                    "a new pack installation cannot start uninstalled".into(),
                ));
            }
            if !names.insert(installation.manifest.name.clone()) {
                return Err(StoreError::Adapter(format!(
                    "duplicate pack in installation batch: {}",
                    installation.manifest.name
                )));
            }
            if let Some(existing) = self.reduce_pack(&installation.manifest.name)?
                && existing.status != PackStatus::Uninstalled
            {
                return Err(StoreError::Adapter(format!(
                    "pack {} is already installed",
                    installation.manifest.name
                )));
            }
            let stream_id = Self::pack_stream(&installation.manifest.name);
            let expected_stream_version = self.journal.read_stream(&stream_id)?.len() as u64;
            events.push(NewEvent {
                event_version: 1,
                stream_id,
                expected_stream_version,
                classification: EventClassification::Domain,
                event_type: "pack.installed.v1".into(),
                actor: actor.clone(),
                context: ExecutionContext {
                    correlation_id: format!("pack-collection:{}", installation.manifest.name),
                    ..ExecutionContext::default()
                },
                payload: serde_json::to_value(installation).map_err(adapter)?,
            });
        }
        self.journal.append_batch(events)?;
        Ok(installations)
    }

    fn set_pack_status(
        &self,
        name: &str,
        status: PackStatus,
        actor: Actor,
        updated_at: &str,
    ) -> Result<PackInstallation, StoreError> {
        validate_name(name)?;
        let mut installation = self
            .reduce_pack(name)?
            .ok_or_else(|| StoreError::NotFound(format!("pack {name}")))?;
        if installation.status == PackStatus::Uninstalled {
            return Err(StoreError::Adapter(format!(
                "pack {name} has already been uninstalled"
            )));
        }
        installation.status = status;
        installation.updated_at = updated_at.into();
        let event_type = match status {
            PackStatus::Enabled => "pack.enabled.v1",
            PackStatus::Disabled => "pack.disabled.v1",
            PackStatus::Uninstalled => "pack.uninstalled.v1",
        };
        self.append_pack(&installation, actor, event_type)?;
        Ok(installation)
    }

    fn add_publisher_trust(
        &self,
        trust: PublisherTrust,
        actor: Actor,
    ) -> Result<PublisherTrust, StoreError> {
        validate_name(&trust.publisher)?;
        if self
            .reduce_trust(&trust.publisher, &trust.key_id)?
            .is_some()
        {
            return Err(StoreError::Adapter(format!(
                "publisher trust {}:{} already exists",
                trust.publisher, trust.key_id
            )));
        }
        let stream_id = Self::trust_stream(&trust.publisher, &trust.key_id);
        self.journal.append(NewEvent {
            event_version: 1,
            stream_id,
            expected_stream_version: 0,
            classification: EventClassification::Domain,
            event_type: "publisher.trusted.v1".into(),
            actor,
            context: ExecutionContext {
                correlation_id: format!("publisher-trust:{}", trust.publisher),
                ..ExecutionContext::default()
            },
            payload: serde_json::to_value(&trust).map_err(adapter)?,
        })?;
        Ok(trust)
    }

    fn get_publisher_trust(
        &self,
        publisher: &str,
        key_id: &str,
    ) -> Result<Option<PublisherTrust>, StoreError> {
        validate_name(publisher)?;
        self.reduce_trust(publisher, key_id)
    }

    fn list_publisher_trust(&self, limit: usize) -> Result<Vec<PublisherTrust>, StoreError> {
        if limit == 0 || limit > MAX_CONNECTIONS {
            return Err(StoreError::Adapter(
                "publisher trust list limit must be in 1..=1000".into(),
            ));
        }
        self.stream_names("publisher-trust:")?
            .into_iter()
            .take(limit)
            .filter_map(|suffix| {
                let (publisher, key_id) = suffix.rsplit_once(':')?;
                self.reduce_trust(publisher, key_id).transpose()
            })
            .collect()
    }
}
