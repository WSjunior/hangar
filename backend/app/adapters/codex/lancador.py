"""O contrato do pane de uma sessao Codex: quem sobe a TUI, com que permissao, e como se apresenta.

Existe separado do `adapter.py` porque TRES processos diferentes precisam concordar nisto e nenhum
deles pode pagar o preco de importar o adapter (asyncio, websockets, parser do transcript):

- o backend, que cria a sessao (`registry.create` -> `CodexAdapter.spawn_command`);
- `scripts/hangar-codex-tui`, o lancador, que roda no pane a cada abertura de sessao;
- `scripts/hangar-codex`, o wrapper do `codex` do shell, que precisa criar a sessao mesmo com o
  backend desligado.

Divergir aqui nao daria erro nenhum — daria uma TUI com permissao diferente da que o resto do
sistema supoe, calada. Modulo so de stdlib, de proposito: o wrapper roda no `python3` do sistema.
"""

# clientInfo do handshake initialize (ver docs/codex-app-server-contract.md).
CLIENT_INFO = {"name": "hangar", "title": None, "version": "0.1.0"}
# Acesso total, como as outras sessoes do app. Os outros tres agentes (Claude, Pi, Kimi) rodam sem
# sandbox nenhum; o Codex era o unico presso, e nao por decisao de quem usa -- foi so o spike que
# parou no meio do caminho (read-only -> workspace-write -> e ficou ali).
# O que o `workspace-write` custava, medido em 30/08/2026: ele corta a REDE, loopback incluido, e
# por isso `hangar-send` de dentro de uma sessao Codex morria em "backend inacessivel em
# 127.0.0.1:8765" com o backend no ar -- o Codex era o unico agente que nao conseguia falar com as
# sessoes irmas. `danger-full-access` e o par do `bypassPermissions` do Claude, e vale nas DUAS
# pontas: e valor aceito pelo `--sandbox` do CLI e pelo campo `sandbox` do app-server (a flag
# `--dangerously-bypass-approvals-and-sandbox` so existe no CLI, e deixaria as duas divergentes).
SANDBOX = "danger-full-access"
APPROVAL = "never"
PERMISSION_POLICIES = {
    "Ask for approval": ("on-request", "read-only"),
    "Approve for me": ("on-request", "workspace-write"),
    "Full Access": ("never", "danger-full-access"),
}
# Nome do lancador no PATH (symlink do install-claude-wrapper.sh).
EXECUTAVEL = "hangar-codex-tui"


def comando_do_lancador(cwd: str, initial_prompt: str | None = None,
                        thread_id: str | None = None, model: str | None = None,
                        effort: str | None = None,
                        codex_home: str | None = None,
                        codex_account: str | None = None,
                        approval: str | None = None, sandbox: str | None = None,
                        tool_output_token_limit: int | None = None,
                        transfer_id: str | None = None,
                        service_tier: str | None = None) -> list[str]:
    """O comando do pane de uma sessao Codex: o lancador unico, o MESMO nos tres chamadores.

    O nome da sessao nao entra aqui — `tmux new-session` carimba CP_SESSION_NAME no pane e o
    lancador le de la. Assim o comando nao repete a identidade que o tmux ja garante.

    `thread_id` retoma uma conversa que ja existe (o "Retomar" do Arquivo) em vez de abrir uma nova.

    `model`/`effort` viajam pro lancador em vez de virar flag aqui porque a traducao e diferente
    das dos outros agentes: o modelo e `-m`, mas o esforco NAO e flag do binario — vai como
    sobrescrita de configuracao (`-c model_reasoning_effort=`). Quem faz essa traducao e o lancador,
    que e quem monta o argv do `codex`.
    """
    if service_tier is not None and service_tier not in ("default", "priority"):
        raise ValueError("service_tier: use default ou priority")
    argv = [EXECUTAVEL, "--cwd", cwd]
    if service_tier is not None:
        argv += ["--service-tier", service_tier]
    if codex_home:
        argv += ["--codex-home", codex_home]
    if codex_account:
        argv += ["--codex-account", codex_account]
    if thread_id:
        argv += ["--resume", thread_id]
    if model:
        argv += ["--model", model]
    if effort:
        argv += ["--effort", effort]
    if approval:
        argv += ["--approval-policy", approval]
    if sandbox:
        argv += ["--sandbox", sandbox]
    if tool_output_token_limit is not None:
        argv += tool_output_override(tool_output_token_limit, launcher=True)
    if transfer_id is not None:
        import uuid
        argv += ["--transfer-id", str(uuid.UUID(transfer_id))]
    if initial_prompt:
        argv += ["--prompt", initial_prompt]
    return argv


def service_tier_override(service_tier: str | None) -> list[str]:
    if service_tier is None:
        return []
    import json
    return ["-c", f"service_tier={json.dumps(service_tier)}"]


def tool_output_override(limit: int | None, *, launcher: bool = False) -> list[str]:
    if limit is None:
        return []
    if type(limit) is not int or limit <= 0:
        raise ValueError("limite de resultado de ferramenta inválido")
    return (["--tool-output-token-limit", str(limit)] if launcher else
            ["-c", f"tool_output_token_limit={limit}"])
