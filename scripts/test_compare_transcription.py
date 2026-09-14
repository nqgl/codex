import asyncio
import base64
import json
from pathlib import Path
import tempfile
import unittest
import wave
from unittest.mock import patch

import compare_transcription as compare


class AudioTests(unittest.TestCase):
    def test_pcm_validation_and_duration(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "clip.wav"
            for rate, frames, valid in [
                (24000, 4800, True),
                (16000, 4800, False),
                (24000, 1, False),
            ]:
                with wave.open(str(path), "wb") as out:
                    out.setnchannels(1)
                    out.setsampwidth(2)
                    out.setframerate(rate)
                    out.writeframes(b"\0\0" * frames)
                if valid:
                    self.assertEqual(compare.load_audio(path), b"\0\0" * frames)
                else:
                    with self.assertRaises(ValueError):
                        compare.load_audio(path)


class FakeSocket:
    def __init__(self, failure=False):
        self.events = asyncio.Queue()
        self.messages = []
        self.failure = failure

    async def send(self, message):
        message = json.loads(message)
        self.messages.append(message)
        if self.failure:
            await self.events.put({"type": "error", "error": {"message": "rejected"}})
        elif message["type"] == "input_audio_buffer.append" and len(self.messages) == 1:
            await self.events.put(
                {
                    "type": "conversation.item.input_audio_transcription.delta",
                    "delta": "hello",
                }
            )
        elif message["type"] == "input_audio_buffer.commit":
            await self.events.put(
                {
                    "type": "conversation.item.input_audio_transcription.completed",
                    "transcript": "Hello.",
                }
            )

    async def recv(self):
        return json.dumps(await self.events.get())


class StreamingTests(unittest.IsolatedAsyncioTestCase):
    async def test_stream_preserves_audio_and_commits_once(self):
        audio = b"\x01\x02" * 4800
        ws = FakeSocket()
        result = await compare.transcribe(ws, audio)
        self.assertEqual(
            [message["type"] for message in ws.messages],
            [
                "input_audio_buffer.append",
                "input_audio_buffer.append",
                "input_audio_buffer.commit",
            ],
        )
        sent = b"".join(
            base64.b64decode(message["audio"]) for message in ws.messages[:-1]
        )
        self.assertEqual(sent, audio)
        self.assertEqual(result["transcript"], "Hello.")
        self.assertTrue(result["text_arrived_before_commit"])
        self.assertGreaterEqual(result["final_seconds_after_commit"], 0)

    async def test_error_stops_audio_sender(self):
        ws = FakeSocket(failure=True)
        with self.assertRaisesRegex(RuntimeError, "rejected"):
            await compare.transcribe(ws, b"\0\0" * 24000)
        sent = list(ws.messages)
        await asyncio.sleep(0.15)
        self.assertEqual(ws.messages, sent)

    async def test_one_model_failure_does_not_hide_the_other_result(self):
        async def run(model, *args):
            return {
                "model": model,
                **(
                    {"error": "unavailable"}
                    if model == compare.MODELS[0]
                    else {"transcript": "hello"}
                ),
            }

        with (
            patch.object(compare, "run_model", side_effect=run),
            patch("builtins.print") as output,
        ):
            status = await compare.compare("unused", b"\0\0", "", "medium")
        self.assertEqual(status, 1)
        self.assertEqual(
            json.loads(output.call_args.args[0])["results"],
            [
                {"model": "gpt-transcribe", "error": "unavailable"},
                {"model": "gpt-live-transcribe", "transcript": "hello"},
            ],
        )


if __name__ == "__main__":
    unittest.main()
