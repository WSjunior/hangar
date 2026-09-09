"""Identidade das skills nativas do Codex."""
from collections import Counter


def skills_do_catalogo(result: dict) -> list[dict]:
    skills = {s["path"]: s for group in result.get("data", []) for s in group.get("skills", [])
              if s.get("enabled") and s.get("name") and s.get("path")}
    counts = Counter(s["name"] for s in skills.values())
    out = []
    for path, skill in skills.items():
        name = skill["name"]
        if counts[name] > 1:
            # Homônimas precisam de identidade estável; nunca escolher um arquivo por acaso.
            import hashlib
            name = f"{name}:{hashlib.sha256(path.encode()).hexdigest()[:8]}"
        out.append({"name": name, "native_name": skill["name"], "path": path,
                    "display": "/" + name, "description": skill.get("description"),
                    "source": "skill", "destructive": False})
    return sorted(out, key=lambda s: s["name"])
