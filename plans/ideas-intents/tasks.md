G1: Implement the end-to-end desktop feature and tests. Key: ideas-intents/G1/1. G2: Independent review after G1.

## Proof
- From the Sidebar, save a typed idea without a title, project or chat; newest first.
- Start and stop capture explicitly. Replay the original audio after restart; see the transcript alongside when local transcription succeeds.
- Missing model, denied microphone and transcription failure have intelligible states; transcription failure never discards saved audio.
- Exercise the real route in an isolated offscreen instance with synthetic audio; distinguish physical microphone/playback from automated checks. No installation or deployment.

G3 corrections after G2: prove Save failure retains the draft and subsequent edits. Leaving Ideas must stop and save active capture, and cancel any still-pending mic start; no hidden recording. Reproduce before the change and after in an isolated local Podman window. Only local Podman for builds/tests. Done: reports/G3.md.
