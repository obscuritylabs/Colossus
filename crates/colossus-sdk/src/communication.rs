use crate::*;

impl Colossus {
    /// Submit peer task input without automatically retrying unknown outcomes.
    pub async fn submit_agent_task_message(
        &self,
        request: SubmitAgentTaskMessageRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        self.require_communication(AGENT_COMMUNICATION_SEND_CAPABILITY)?;
        self.agent_runs().submit_agent_task_message(request).await
    }
    /// Read a curated caller-owned task with bounded peer input history.
    pub async fn get_agent_task(
        &self,
        request: GetAgentTaskRequest,
    ) -> ApiResult<AgentTaskSnapshot> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().get_agent_task(request).await
    }
    /// Query canonical tasks ordered by last lifecycle update.
    pub async fn list_agent_tasks(
        &self,
        request: ListAgentTasksRequest,
    ) -> ApiResult<ListAgentTasksResponse> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().list_agent_tasks(request).await
    }

    /// Inspect caller-owned exact attempt addresses, including closed inboxes.
    pub async fn list_agent_participants(
        &self,
        request: ListAgentParticipantsRequest,
    ) -> ApiResult<Vec<AgentParticipant>> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().list_agent_participants(request).await
    }
    /// Send bounded peer input; never automatically retry a mutation with unknown outcome.
    pub async fn send_agent_message(&self, request: SendAgentMessage) -> ApiResult<AgentMessage> {
        self.require_communication(AGENT_COMMUNICATION_SEND_CAPABILITY)?;
        self.agent_runs().send_agent_message(request).await
    }
    /// Inspect the current durable receipt for one admitted message.
    pub async fn get_agent_message(
        &self,
        request: GetAgentMessageRequest,
    ) -> ApiResult<AgentMessage> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().get_agent_message(request).await
    }
    /// Read one bounded recipient-ordered page of released message history.
    pub async fn list_agent_messages(
        &self,
        request: ListAgentMessagesRequest,
    ) -> ApiResult<AgentMessagePage> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().list_agent_messages(request).await
    }
    /// Replay and tail the independent communication feed after an exclusive cursor.
    pub async fn watch_agent_messages(
        &self,
        request: WatchAgentMessagesRequest,
    ) -> ApiResult<AgentCommunicationStream> {
        self.require_communication(AGENT_COMMUNICATION_READ_CAPABILITY)?;
        self.agent_runs().watch_agent_messages(request).await
    }
    fn require_communication(&self, capability: &str) -> ApiResult<()> {
        if self.capabilities().contains(capability) {
            Ok(())
        } else {
            Err(colossus_api::agent_communication_unavailable())
        }
    }
}
