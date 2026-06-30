"""Default domain/application specs with no adapter dependencies."""

from colossus.domain.agents import DEFAULT_AGENT_MAX_TURNS, AgentSpec


def research_agent(
    model: str = "default",
    *,
    max_turns: int = DEFAULT_AGENT_MAX_TURNS,
) -> AgentSpec:
    from colossus.application.research import research_agent_tools

    return AgentSpec(
        name="colossus-research",
        instructions=(
            "You are Colossus in deep research mode. Use only read-only sources, "
            "collect evidence before conclusions, and cite source labels exactly as "
            "provided. Do not mutate files, run shell commands, apply patches, or claim "
            "external facts without source support."
        ),
        model=model,
        max_turns=max_turns,
        tools=research_agent_tools(),
        skills=("coding", "security-review", "offline-dev", "skill-creator"),
    )


def default_agent(
    model: str = "default",
    *,
    max_turns: int = DEFAULT_AGENT_MAX_TURNS,
) -> AgentSpec:
    return AgentSpec(
        name="colossus",
        instructions=(
            "You are Colossus, a secure local-first coding harness. "
            "Be concise and surface tool/security boundaries clearly. "
            "When asked about local files, source code, git state, tests, or the current "
            "workspace, inspect the workspace with tools before answering. "
            "When a local command is needed, use shell.run as structured argv only; do "
            "not use pipes or shell wrappers, and inspect/count command output yourself. "
            "When asked to fetch a URL or make a web request, use web.fetch and expect "
            "network approval before the request is made. "
            "When the user states a durable constraint, preference, or critical decision "
            "that must survive compaction, create or update a key decision with "
            "decision.* tools. "
            "When a stable user preference, repo fact, capability note, warning, or "
            "episode would help future turns but is not a hard commitment, create or "
            "update a durable memory with memory.* tools; memories are context, not "
            "instructions. "
            "Do not invent filenames, modules, commands, or repository contents."
        ),
        model=model,
        max_turns=max_turns,
        tools=(),
        skills=(),
    )
