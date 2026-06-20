#!/usr/bin/env python3
"""Serve a cached Hugging Face chat model as OpenAI chat-completions.

This is an operational endpoint helper for local models. It is not a Full State
Verification script; Synapse behavior must still be verified manually through
the real MCP tool surface with separate Source-of-Truth readbacks.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import re
import time
import uuid
from typing import Any

import torch
import uvicorn
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import JSONResponse
from transformers import AutoModelForCausalLM, AutoTokenizer


TOOL_CALL_RE = re.compile(r"<tool_call>\s*(.*?)\s*</tool_call>", re.DOTALL)


def log_event(event: str, **fields: Any) -> None:
    row = {"event": event, **fields}
    print(json.dumps(row, ensure_ascii=True, sort_keys=True), flush=True)


def compact_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=True, separators=(",", ":"))


def normalize_content(content: Any) -> str:
    if content is None:
        return ""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts: list[str] = []
        for part in content:
            if isinstance(part, str):
                parts.append(part)
            elif isinstance(part, dict) and part.get("type") == "text":
                parts.append(str(part.get("text", "")))
        return "\n".join(parts)
    return str(content)


def normalize_messages(raw_messages: Any) -> list[dict[str, Any]]:
    if not isinstance(raw_messages, list):
        raise HTTPException(status_code=400, detail="messages must be an array")
    messages: list[dict[str, Any]] = []
    for index, raw in enumerate(raw_messages):
        if not isinstance(raw, dict):
            raise HTTPException(status_code=400, detail=f"messages[{index}] must be an object")
        role = raw.get("role")
        if not isinstance(role, str) or not role:
            raise HTTPException(status_code=400, detail=f"messages[{index}].role must be a string")
        message: dict[str, Any] = {"role": role, "content": normalize_content(raw.get("content"))}
        for key in ("name", "tool_call_id", "tool_calls"):
            if key in raw:
                message[key] = raw[key]
        messages.append(message)
    return messages


def iter_json_objects(segment: str) -> list[dict[str, Any]]:
    decoder = json.JSONDecoder()
    objects: list[dict[str, Any]] = []
    for match in re.finditer(r"\{", segment):
        try:
            value, _end = decoder.raw_decode(segment[match.start() :])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            objects.append(value)
            break
    return objects


def extract_tool_json(raw_text: str) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    seen: set[str] = set()
    segments = [match.group(1) for match in TOOL_CALL_RE.finditer(raw_text)]
    for close_match in re.finditer(r"</tool_call>", raw_text):
        prefix = raw_text[: close_match.start()]
        start = max(prefix.rfind("<tool_call>"), prefix.rfind("\n{"), prefix.rfind("\r{"))
        if start >= 0:
            segments.append(prefix[start:])
    for segment in segments:
        for value in iter_json_objects(segment):
            key = compact_json(value)
            if key not in seen:
                candidates.append(value)
                seen.add(key)
    return candidates


def tool_call_from_object(index: int, value: dict[str, Any]) -> dict[str, Any] | None:
    name: Any = None
    arguments: Any = {}
    if isinstance(value.get("function"), dict):
        function = value["function"]
        name = function.get("name")
        arguments = function.get("arguments", {})
    elif "name" in value:
        name = value.get("name")
        arguments = value.get("arguments", {})
    if not isinstance(name, str) or not name:
        return None
    if isinstance(arguments, str):
        argument_text = arguments
    else:
        argument_text = compact_json(arguments if arguments is not None else {})
    return {
        "index": index,
        "id": f"call_{uuid.uuid4().hex[:24]}",
        "type": "function",
        "function": {"name": name, "arguments": argument_text},
    }


def parse_tool_calls(raw_text: str) -> list[dict[str, Any]]:
    calls: list[dict[str, Any]] = []
    for value in extract_tool_json(raw_text):
        call = tool_call_from_object(len(calls), value)
        if call is not None:
            calls.append(call)
    return calls


class Endpoint:
    def __init__(self, args: argparse.Namespace) -> None:
        self.args = args
        self.started_at = int(time.time())
        self.lock = asyncio.Lock()
        self.request_count = 0
        log_event("loading_model", model_dir=args.model_dir, model_name=args.model_name)
        self.tokenizer = AutoTokenizer.from_pretrained(
            args.model_dir,
            local_files_only=True,
            trust_remote_code=True,
        )
        load_kwargs: dict[str, Any] = {
            "local_files_only": True,
            "trust_remote_code": True,
            "dtype": args.dtype,
        }
        if args.device_map.lower() not in ("", "none"):
            load_kwargs["device_map"] = args.device_map
        try:
            self.model = AutoModelForCausalLM.from_pretrained(args.model_dir, **load_kwargs)
        except TypeError:
            load_kwargs["torch_dtype"] = load_kwargs.pop("dtype")
            self.model = AutoModelForCausalLM.from_pretrained(args.model_dir, **load_kwargs)
        except ValueError as error:
            if "requires `accelerate`" not in str(error) or "device_map" not in load_kwargs:
                raise
            log_event("device_map_unavailable_fallback", detail=str(error))
            load_kwargs.pop("device_map", None)
            self.model = AutoModelForCausalLM.from_pretrained(args.model_dir, **load_kwargs)
        if not getattr(self.model, "hf_device_map", None) and torch.cuda.is_available():
            self.model.to("cuda")
        self.model.eval()
        self.device = next(self.model.parameters()).device
        log_event(
            "ready",
            model_dir=args.model_dir,
            model_name=args.model_name,
            device=str(self.device),
            cuda=torch.cuda.is_available(),
        )

    def prompt_inputs(self, payload: dict[str, Any]) -> dict[str, torch.Tensor]:
        messages = normalize_messages(payload.get("messages"))
        tools = payload.get("tools")
        kwargs: dict[str, Any] = {
            "conversation": messages,
            "tokenize": True,
            "add_generation_prompt": True,
            "return_tensors": "pt",
            "return_dict": True,
        }
        if isinstance(tools, list) and tools:
            kwargs["tools"] = tools
        if "enable_thinking" in payload:
            kwargs["enable_thinking"] = bool(payload["enable_thinking"])
        try:
            encoded = self.tokenizer.apply_chat_template(**kwargs)
        except TypeError:
            kwargs.pop("tools", None)
            kwargs.pop("enable_thinking", None)
            encoded = self.tokenizer.apply_chat_template(**kwargs)
        return {key: value.to(self.device) for key, value in encoded.items()}

    def generate(self, payload: dict[str, Any]) -> dict[str, Any]:
        max_new_tokens = payload.get("max_tokens", payload.get("max_new_tokens"))
        if max_new_tokens is None:
            max_new_tokens = self.args.max_new_tokens
        max_new_tokens = max(1, min(int(max_new_tokens), self.args.max_new_tokens_limit))
        temperature = float(payload.get("temperature", 0) or 0)
        do_sample = temperature > 0
        inputs = self.prompt_inputs(payload)
        input_ids = inputs["input_ids"]
        prompt_tokens = int(input_ids.shape[-1])
        generate_kwargs: dict[str, Any] = {
            **inputs,
            "max_new_tokens": max_new_tokens,
            "do_sample": do_sample,
        }
        if self.tokenizer.eos_token_id is not None:
            generate_kwargs["pad_token_id"] = self.tokenizer.eos_token_id
        if do_sample:
            generate_kwargs["temperature"] = temperature
            if payload.get("top_p") is not None:
                generate_kwargs["top_p"] = float(payload["top_p"])
        with torch.inference_mode():
            output = self.model.generate(**generate_kwargs)
        generated = output[0][prompt_tokens:]
        raw_text = self.tokenizer.decode(generated, skip_special_tokens=True)
        completion_tokens = int(generated.shape[-1])
        tool_calls = parse_tool_calls(raw_text)
        message: dict[str, Any] = {
            "role": "assistant",
            "content": "" if tool_calls else raw_text,
        }
        finish_reason = "stop"
        if tool_calls:
            message["tool_calls"] = tool_calls
            finish_reason = "tool_calls"
        log_event(
            "completion",
            finish_reason=finish_reason,
            prompt_tokens=prompt_tokens,
            completion_tokens=completion_tokens,
            raw_text=raw_text[:1000],
        )
        return {
            "id": f"chatcmpl-{uuid.uuid4().hex}",
            "object": "chat.completion",
            "created": int(time.time()),
            "model": payload.get("model") or self.args.model_name,
            "choices": [
                {
                    "index": 0,
                    "message": message,
                    "finish_reason": finish_reason,
                }
            ],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            },
        }


def create_app(endpoint: Endpoint) -> FastAPI:
    app = FastAPI()

    @app.get("/health")
    async def health() -> dict[str, Any]:
        return {
            "ok": True,
            "model": endpoint.args.model_name,
            "model_dir": endpoint.args.model_dir,
            "device": str(endpoint.device),
            "queue_locked": endpoint.lock.locked(),
            "request_count": endpoint.request_count,
            "started_at": endpoint.started_at,
        }

    @app.post("/v1/chat/completions")
    async def chat_completions(request: Request) -> JSONResponse:
        payload = await request.json()
        if not isinstance(payload, dict):
            raise HTTPException(status_code=400, detail="request body must be a JSON object")
        if payload.get("stream"):
            raise HTTPException(
                status_code=400,
                detail="stream=true is not supported by this local endpoint",
            )
        endpoint.request_count += 1
        request_id = f"req-{endpoint.request_count}"
        queue_started = time.monotonic()
        try:
            await asyncio.wait_for(
                endpoint.lock.acquire(),
                timeout=endpoint.args.queue_timeout_ms / 1000,
            )
        except asyncio.TimeoutError as exc:
            raise HTTPException(status_code=503, detail="local model queue wait timed out") from exc
        queue_wait_ms = int((time.monotonic() - queue_started) * 1000)
        log_event("request_started", request_id=request_id, queue_wait_ms=queue_wait_ms)
        try:
            result = await asyncio.to_thread(endpoint.generate, payload)
            return JSONResponse(result)
        finally:
            endpoint.lock.release()
            log_event("request_finished", request_id=request_id)

    return app


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-dir", required=True)
    parser.add_argument("--model-name", required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8002)
    parser.add_argument("--device-map", default="auto")
    parser.add_argument("--dtype", default="auto")
    parser.add_argument("--max-new-tokens", type=int, default=256)
    parser.add_argument("--max-new-tokens-limit", type=int, default=2048)
    parser.add_argument("--queue-timeout-ms", type=int, default=120000)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    endpoint = Endpoint(args)
    app = create_app(endpoint)
    uvicorn.run(app, host=args.host, port=args.port, log_level="info")


if __name__ == "__main__":
    main()
