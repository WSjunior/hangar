"""Perguntas nativas, identificadas pelo pedido do servidor e pelo ID de cada pergunta."""


def pending(client, thread_id: str) -> dict | None:
    for request_id, request in getattr(client, "server_requests", {}).items():
        params = request.get("params") or {}
        if request.get("method") != "item/tool/requestUserInput" or params.get("threadId") != thread_id:
            continue
        return {
            "provider": "codex", "request_id": request_id,
            "questions": [
                {"id": q["id"], "header": q["header"], "question": q["question"],
                 "multiSelect": False, "isOther": q.get("isOther", False),
                 "isSecret": q.get("isSecret", False), "options": q.get("options") or []}
                for q in params.get("questions", [])
            ],
        }
    return None


def response(question: dict, answers: list[dict]) -> dict:
    expected = {q["id"]: q for q in question["questions"]}
    result = {}
    for answer in answers:
        qid = answer.get("question_id")
        if qid not in expected or qid in result:
            raise ValueError("A resposta não corresponde às perguntas pendentes.")
        q = expected[qid]
        if answer.get("kind") == "text":
            value = answer.get("value") or ""
            if not value.strip() or (q["options"] and not q["isOther"]):
                raise ValueError("Esta pergunta não aceita essa resposta em texto.")
            values = [value]
        elif answer.get("kind") == "option":
            options = q["options"]
            indices = answer.get("indices") or []
            if len(indices) != 1 or not isinstance(indices[0], int) or not 0 <= indices[0] < len(options):
                raise ValueError("Escolha uma opção válida para cada pergunta.")
            values = [options[indices[0]]["label"]]
        else:
            raise ValueError("Responda à pergunta antes de enviar.")
        result[qid] = {"answers": values}
    if result.keys() != expected.keys():
        raise ValueError("Responda a todas as perguntas antes de enviar.")
    return {"answers": result}
