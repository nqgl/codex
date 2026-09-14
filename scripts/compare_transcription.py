#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# dependencies = ["websockets==11.0.3"]
# ///
"""Compare two OpenAI transcription models on the same paced audio recording.

Run with `uv run scripts/compare_transcription.py --record 20` on Linux, or
pass a 24 kHz, mono, PCM16 WAV with `--wav recording.wav` on any platform.
Requires OPENAI_API_KEY. Sends audio to api.openai.com and incurs API usage.
Does not change Codex settings. Recordings are temporary; transcripts go to stdout.
"""

import argparse
import asyncio
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import wave

import websockets


MODELS = ("gpt-transcribe", "gpt-live-transcribe")
URL = "wss://api.openai.com/v1/realtime?intent=transcription"
SAMPLE_RATE = 24_000
BYTES_PER_SECOND = SAMPLE_RATE * 2
CHUNK_BYTES = BYTES_PER_SECOND // 10
MAX_SECONDS = 120


def load_audio(path: Path) -> bytes:
    with wave.open(str(path), "rb") as recording:
        if (
            recording.getnchannels(),
            recording.getsampwidth(),
            recording.getframerate(),
            recording.getcomptype(),
        ) != (1, 2, SAMPLE_RATE, "NONE"):
            raise ValueError("Use an uncompressed 24 kHz mono PCM16 WAV.")
        frames = recording.getnframes()
        if not SAMPLE_RATE // 5 <= frames <= SAMPLE_RATE * MAX_SECONDS:
            raise ValueError(f"Use between 0.2 and {MAX_SECONDS} seconds of audio.")
        audio = recording.readframes(frames)
        if len(audio) != frames * 2:
            raise ValueError("The WAV is truncated.")
        return audio


def session_update(model: str, prompt: str, delay: str) -> dict:
    transcription = {"model": model}
    if prompt:
        transcription["prompt"] = prompt
    if model == "gpt-live-transcribe":
        transcription["delay"] = delay
    return {
        "type": "session.update",
        "session": {
            "type": "transcription",
            "audio": {
                "input": {
                    "format": {"type": "audio/pcm", "rate": SAMPLE_RATE},
                    "transcription": transcription,
                    # One explicit commit makes both models transcribe identical turns.
                    "turn_detection": None,
                }
            },
        },
    }


async def transcribe(ws, audio: bytes) -> dict:
    started = time.monotonic()
    first_delta_at = None
    committed_at = None

    async def send_audio():
        nonlocal committed_at
        for offset in range(0, len(audio), CHUNK_BYTES):
            await asyncio.sleep(
                max(0, started + offset / BYTES_PER_SECOND - time.monotonic())
            )
            await ws.send(
                json.dumps(
                    {
                        "type": "input_audio_buffer.append",
                        "audio": base64.b64encode(
                            audio[offset : offset + CHUNK_BYTES]
                        ).decode("ascii"),
                    }
                )
            )
        await asyncio.sleep(
            max(0, started + len(audio) / BYTES_PER_SECOND - time.monotonic())
        )
        committed_at = time.monotonic()
        await ws.send(json.dumps({"type": "input_audio_buffer.commit"}))

    async def receive_transcript():
        nonlocal first_delta_at
        while True:
            event = json.loads(await ws.recv())
            kind = event.get("type")
            if kind in ("error", "conversation.item.input_audio_transcription.failed"):
                raise RuntimeError(json.dumps(event.get("error", {})))
            if (
                kind == "conversation.item.input_audio_transcription.delta"
                and event.get("delta")
            ):
                if first_delta_at is None:
                    first_delta_at = time.monotonic()
            if kind == "conversation.item.input_audio_transcription.completed":
                finished = time.monotonic()
                if committed_at is None:
                    raise RuntimeError(
                        "Received a final transcript before the explicit commit."
                    )
                return {
                    "transcript": event.get("transcript", ""),
                    "first_text_seconds_from_audio_start": (
                        round(first_delta_at - started, 3)
                        if first_delta_at is not None
                        else None
                    ),
                    "text_arrived_before_commit": (
                        first_delta_at < committed_at
                        if first_delta_at is not None
                        else False
                    ),
                    "final_seconds_after_commit": round(finished - committed_at, 3),
                    "total_seconds_from_audio_start": round(finished - started, 3),
                    "usage": event.get("usage"),
                }

    sender = asyncio.create_task(send_audio())
    receiver = asyncio.create_task(receive_transcript())
    try:
        _, result = await asyncio.wait_for(
            asyncio.gather(sender, receiver), timeout=len(audio) / BYTES_PER_SECOND + 45
        )
        return result
    finally:
        for task in (sender, receiver):
            task.cancel()
        await asyncio.gather(sender, receiver, return_exceptions=True)


async def run_model(
    model: str, key: str, audio: bytes | None, prompt: str, delay: str
) -> dict:
    started = time.monotonic()
    try:
        async with websockets.connect(
            URL,
            extra_headers={"Authorization": f"Bearer {key}"},
            open_timeout=15,
            close_timeout=3,
            max_size=2**20,
        ) as ws:
            await ws.send(json.dumps(session_update(model, prompt, delay)))
            async with asyncio.timeout(20):
                while True:
                    event = json.loads(await ws.recv())
                    if event.get("type") == "error":
                        raise RuntimeError(json.dumps(event.get("error", {})))
                    if event.get("type") == "session.updated":
                        actual = event["session"]["audio"]["input"]["transcription"][
                            "model"
                        ]
                        if actual != model:
                            raise RuntimeError(
                                f"Requested {model}, but server selected {actual}."
                            )
                        break
            result = {
                "model": model,
                "setup_seconds": round(time.monotonic() - started, 3),
            }
            if audio is None:
                result["status"] = "session accepted; no audio sent"
            else:
                result.update(await transcribe(ws, audio))
            return result
    except Exception as error:
        # No headers, credentials, or full wire messages in diagnostics.
        return {"model": model, "error": str(error).replace(key, "[redacted]")[:1500]}


async def compare(key: str, audio: bytes | None, prompt: str, delay: str) -> int:
    results = await asyncio.gather(
        *(run_model(model, key, audio, prompt, delay) for model in MODELS)
    )
    print(
        json.dumps(
            {
                "audio_seconds": len(audio) / BYTES_PER_SECOND
                if audio is not None
                else None,
                "live_delay": delay,
                "context_prompt_supplied": bool(prompt),
                "results": results,
            },
            indent=2,
        )
    )
    return int(any("error" in result for result in results))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--wav", type=Path, help="Send this recording to both models")
    source.add_argument(
        "--record", type=int, metavar="SECONDS", help="Record with arecord (Linux)"
    )
    source.add_argument(
        "--probe", action="store_true", help="Check session access without audio"
    )
    parser.add_argument("--device", default="default", help="ALSA recording device")
    parser.add_argument(
        "--prompt", default="", help="Optional identical context hint for both models"
    )
    parser.add_argument(
        "--delay",
        choices=["minimal", "low", "medium", "high", "xhigh"],
        default="medium",
    )
    args = parser.parse_args()
    key = os.environ.get("OPENAI_API_KEY")
    if not key:
        parser.error(
            "Set OPENAI_API_KEY in your environment; do not pass it on the command line."
        )
    if args.record is not None and not 1 <= args.record <= MAX_SECONDS:
        parser.error(f"--record must be between 1 and {MAX_SECONDS} seconds.")
    try:
        audio = None
        if args.wav:
            audio = load_audio(args.wav)
        elif args.record is not None:
            recorder = shutil.which("arecord")
            if recorder is None:
                parser.error(
                    "Recording requires arecord on Linux. Otherwise use --wav."
                )
            print(
                f"Record {args.record} seconds, then send it to both models at api.openai.com. API charges apply.",
                file=sys.stderr,
            )
            input("Press Enter to begin recording (Ctrl-C cancels): ")
            with tempfile.TemporaryDirectory(
                prefix="codex-transcription-"
            ) as directory:
                path = Path(directory) / "sample.wav"
                subprocess.run(
                    [
                        recorder,
                        "-q",
                        "-D",
                        args.device,
                        "-t",
                        "wav",
                        "-f",
                        "S16_LE",
                        "-r",
                        str(SAMPLE_RATE),
                        "-c",
                        "1",
                        "-d",
                        str(args.record),
                        str(path),
                    ],
                    check=True,
                    timeout=args.record + 10,
                )
                audio = load_audio(path)
        return asyncio.run(compare(key, audio, args.prompt, args.delay))
    except (OSError, ValueError, wave.Error, subprocess.SubprocessError) as error:
        parser.exit(1, f"{error}\n")
    except (KeyboardInterrupt, EOFError):
        parser.exit(130, "Cancelled.\n")


if __name__ == "__main__":
    sys.exit(main())
