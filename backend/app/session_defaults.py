"""Escolha do provedor quando a abertura não especifica um."""


def choose_provider(probes: dict[str, dict], remembered: str | None,
                    connected: set[str], disconnected: set[str]) -> str:
    available = [provider for provider, probe in probes.items() if probe.get("disponivel")]
    if remembered in available and (remembered not in disconnected or not connected):
        return remembered
    return next((provider for provider in available if provider in connected),
                next(iter(available), "claude"))
