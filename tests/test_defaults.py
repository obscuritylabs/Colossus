from colossus.application.defaults import default_agent, research_agent


def test_default_agent_explains_structured_shell_usage() -> None:
    instructions = default_agent().instructions

    assert "shell.run" in instructions
    assert "structured argv" in instructions
    assert "do not use pipes or shell wrappers" in instructions


def test_default_agent_explains_key_decision_usage() -> None:
    instructions = default_agent().instructions

    assert "key decision" in instructions
    assert "survive compaction" in instructions
    assert "decision.* tools" in instructions


def test_default_agent_empty_tools_means_full_catalog() -> None:
    assert default_agent().tools == ()


def test_research_agent_uses_read_only_tool_subset() -> None:
    agent = research_agent()

    assert "filesystem.read" in agent.tools
    assert "repo.map" in agent.tools
    assert "web.search" in agent.tools
    assert "mcp.call" in agent.tools
    assert "filesystem.write" not in agent.tools
    assert "shell.run" not in agent.tools
