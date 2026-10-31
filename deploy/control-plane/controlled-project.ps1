# Shared controlled Hosted template for deployment adapters. Runtime owns its ceiling.
function New-ColossusControlledProject {
    # Matches RuntimeConfig::controlled_chat_template and the supported Hosted
    # example. Echo is a usable offline default; the ceiling still supports later
    # explicitly configured providers, MCP and search without redeployment.
    $tools = @('echo', 'user.ask*', 'tool.search', 'context.show', 'context.compact', 'context.snapshots', 'context.restore', 'trace.show',
        'task.create', 'task.update', 'task.list', 'decision.create', 'decision.update', 'decision.list', 'decision.archive', 'decision.supersede',
        'session.set_title', 'plan.create', 'plan.update', 'goal.show', 'agent.delegate', 'agent.participants', 'agent.send_message', 'agent.inbox',
        'agent.await_message', 'agent.result', 'agent.list', 'goal.update', 'plan.show', 'plan.approve_request', 'memory.create', 'memory.update',
        'memory.list', 'memory.search', 'memory.archive', 'memory.supersede', 'mcp.servers*', 'mcp.search*', 'mcp.tools*', 'mcp.call*', 'web.search*',
        'workflow.definition.list', 'workflow.definition.get', 'workflow.schedule.list', 'workflow.schedule.get')
    $actions = @('agent.message.read', 'agent.message.send', 'agent.run', 'context.compact', 'context.restore', 'context.show', 'context.snapshots',
        'decision.archive', 'decision.create', 'decision.list', 'decision.supersede', 'decision.update', 'goal.create', 'goal.iteration.record',
        'goal.show', 'goal.update', 'mcp.invoke', 'mcp.tools', 'memory.archive', 'memory.create', 'memory.index.rebuild', 'memory.index.status',
        'memory.index.sync', 'memory.list', 'memory.read', 'memory.search', 'memory.supersede', 'memory.update', 'plan.create', 'plan.discard',
        'plan.execute', 'plan.show', 'plan.update', 'presentation.history.append', 'presentation.preferences.update', 'provider.anthropic.messages',
        'provider.call', 'provider.echo', 'provider.models', 'provider.openai.chat', 'provider.openai.codex', 'provider.openai.responses', 'research.run',
        'session.set_title', 'subagent.cancel', 'subagent.complete', 'subagent.create', 'subagent.fail', 'subagent.interrupt', 'subagent.list',
        'subagent.read', 'subagent.requeue', 'subagent.start', 'task.create', 'task.list', 'task.update', 'web.search', 'workflow.definition.read',
        'workflow.definition.register', 'workflow.execute', 'workflow.run.read', 'workflow.run.start', 'workflow.schedule.get', 'workflow.schedule.list', 'workflow.start')
    return @{
        runtime = @{
            schemaVersion = 3
            access = @{ profile = 'pinned'; tools = @{ include = $tools; exclude = @() }; actions = @{ allow = $actions; requireApproval = @('mcp.call', 'plan.approve_request'); deny = @() } }
            storage = @{ location = 'workspace'; path = '/var/lib/colossus-chat/private/template.redb'; adapter = 'redb'; startupVerification = 'incremental'
                keys = @{ kind = 'environment'; journal_variable = 'COLOSSUS_CHAT_JOURNAL_KEY'; journal_key_id = 'hosted-chat-template'; signing_variable = 'COLOSSUS_CHAT_SIGNING_KEY'; anchor_path = '/var/lib/colossus-chat/private/template.anchor' } }
            network = @{ maxRedirects = 10; caBundlePath = $null; clientCertificatePath = $null; clientKeyPath = $null }
            policy = @{ kind = 'built_in'; require_post_effect = $false }
            providers = @{ profiles = @{ main = @{ kind = 'echo' } } }
            models = @{ profiles = @{ main = @{ providerProfile = 'main'; model = 'echo'; contextWindowTokens = 32768; maxOutputTokens = 4096
                capabilities = @{ toolCalls = 'on'; streaming = 'on'; imageInputs = 'off'; serverCompaction = 'auto'; declared = @{ toolCalls = $null; streaming = $null; imageInputs = $null; serverCompaction = $null } } } }; roles = @{ primary = 'main' } }
            mcp = @{ oauthCredentialStore = 'platform'; servers = @{} }
            sandbox = @{ backend = 'native'; profile = 'offline-default'; allowBrokerFallback = $false; acknowledgeExternalBoundary = $false; acknowledgeDangerFullAccess = $false
                helperPath = $null; ociRuntime = $null; ociImage = $null; ociProxyImage = $null; filesystem = @(); executables = @(); environment = @(); networkDestinations = @()
                timeoutMs = 900000; maxOutputBytes = 4194304; maxProcesses = 16; maxMemoryBytes = 1073741824; maxConcurrency = 1 }
            browser = @{ enabled = $false }; computer = @{ enabled = $false }; plugins = @{ enabled = $false; workspaceDiscovery = $false }
            agent = @{ maxTurns = 8 }; subagents = @{ maxConcurrent = 2 }
        }
        provider_authentication = @{ main = 'none' }; mcp_authentication = @{}; search_authentication = @{}; provider_generations = @{ main = 1 }; mcp_generations = @{}
        revision = 1; codex_auth_path = $null; codex_auth_shared = $false
    }
}
