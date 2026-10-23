# /// script
# requires-python = ">=3.11,<3.12"
# dependencies = [
#     "chatterbox-tts==0.1.7",
#     "setuptools<80",
#     "fastapi==0.161.1",
#     "uvicorn==0.54.0",
# ]
# ///
"""An OpenAI-compatible speech server for Maya, built on Chatterbox Turbo.

    uv run chatterbox/server.py [--host 127.0.0.1] [--port 8004]
                                [--device cpu|mps|cuda] [--voices <dir>]

Set CHATTERBOX_API_KEY to require `Authorization: Bearer <key>`. The key comes
from the environment only, so it never shows up in `ps`.

Voices: `default` is the model's built-in voice. Every .wav, .mp3 or .flac file
in the voices folder (longer than 5 seconds) is a voice named after the file.
"""

import argparse
import hmac
import io
import os
import threading
import wave
from pathlib import Path

import uvicorn
from fastapi import Body, FastAPI, Header
from fastapi.responses import JSONResponse, Response

CLIP_EXTENSIONS = {".wav", ".mp3", ".flac"}
DEFAULT_VOICE = "default"

app = FastAPI()
api_key = os.environ.get("CHATTERBOX_API_KEY", "")
voices_dir = Path(__file__).resolve().parent / "voices"
model = None
default_conds = None
# One generation at a time: the model holds its current voice in model.conds.
lock = threading.Lock()
# voice name -> (file mtime, prepared conditionals)
prepared = {}


def error(status, message):
    return JSONResponse({"error": message}, status_code=status)


def unauthorized(authorization):
    if not api_key:
        return False
    given = (authorization or "").removeprefix("Bearer ").strip()
    return not hmac.compare_digest(given.encode(), api_key.encode())


def voice_files():
    """The clips in the voices folder by name, read fresh on every call."""
    if not voices_dir.is_dir():
        return {}
    return {
        p.name: p
        for p in sorted(voices_dir.iterdir())
        if p.is_file() and p.suffix.lower() in CLIP_EXTENSIONS
    }


def conditionals_for(name, files):
    """The prepared conditionals for a voice; raises AssertionError for a clip
    Chatterbox rejects (too short). Call with the lock held."""
    if name in ("", DEFAULT_VOICE):
        return default_conds
    path = files[name]
    mtime = path.stat().st_mtime
    cached = prepared.get(name)
    if cached is None or cached[0] != mtime:
        model.prepare_conditionals(str(path))
        cached = prepared[name] = (mtime, model.conds)
    return cached[1]


def wav_bytes(samples, rate):
    """16-bit mono PCM from a float tensor shaped (1, samples)."""
    data = samples.squeeze().clamp(-1.0, 1.0).mul(32767).short().cpu().numpy()
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(data.tobytes())
    return out.getvalue()


@app.get("/v1/audio/voices")
def voices(authorization: str | None = Header(None)):
    if unauthorized(authorization):
        return error(401, "unauthorized")
    return {"status": "ok", "voices": [DEFAULT_VOICE, *voice_files()]}


@app.post("/v1/audio/speech")
def speech(body: dict = Body(...), authorization: str | None = Header(None)):
    if unauthorized(authorization):
        return error(401, "unauthorized")
    text = body.get("input")
    if not isinstance(text, str) or not text.strip():
        return error(400, "input is required")
    if body.get("response_format") not in (None, "wav"):
        return error(400, "only wav is supported")
    name = (body.get("voice") or "").strip()
    files = voice_files()
    if name not in ("", DEFAULT_VOICE) and name not in files:
        return error(404, f"unknown voice: {name}")
    with lock:
        try:
            model.conds = conditionals_for(name, files)
        except AssertionError as e:
            return error(400, str(e) or "the voice clip is not usable")
        audio = model.generate(text)
        data = wav_bytes(audio, model.sr)
    return Response(data, media_type="audio/wav")


def main():
    global model, default_conds, voices_dir
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8004)
    parser.add_argument("--device", default="cpu", choices=["cpu", "mps", "cuda"])
    parser.add_argument("--voices", type=Path, default=voices_dir)
    args = parser.parse_args()
    voices_dir = args.voices.resolve()
    if args.device == "mps":
        os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")

    from chatterbox.tts_turbo import ChatterboxTurboTTS

    model = ChatterboxTurboTTS.from_pretrained(device=args.device)
    default_conds = model.conds
    needs = "an API key is required" if api_key else "no API key"
    print(f"Chatterbox server at http://{args.host}:{args.port} on {args.device}, {needs}", flush=True)
    uvicorn.run(app, host=args.host, port=args.port, log_level="warning")


if __name__ == "__main__":
    main()
