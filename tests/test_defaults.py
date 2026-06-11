from colossus.application.defaults import default_agent


def test_default_agent_explains_structured_shell_usage() -> None:
    instructions = default_agent().instructions

    assert "shell.run" in instructions
    assert "structured argv" in instructions
    assert "do not use pipes or shell wrappers" in instructions
