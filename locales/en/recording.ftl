# Recording a region of the screen to a video. $reason: what went wrong, as the system says it.

## Why a recording didn't start, or ended by itself

record-error-two-monitors = The box is on two monitors. A recording is of one monitor: move the box onto one.
record-error-off-screen = The box isn't on any monitor.
# $min: the smallest side allowed, in pixels.
record-error-too-small = The box is too small to record: each side needs at least { $min } pixels on its monitor.
record-error-no-media-foundation = Videos can't be recorded: this copy of Windows has no Media Foundation. On Windows N editions, install the Media Feature Pack from Settings, Optional features.
# $width, $height: the video's size in pixels.
record-error-encoder-refused = The video encoder won't record a box { $width } x { $height } pixels: { $reason }
record-error-capture-unavailable = The screen can't be recorded just now: { $reason }
record-error-capture-lost = The recording stopped because the screen couldn't be captured any more: { $reason }. What was recorded until then is saved.
record-error-encoding = The video couldn't be encoded: { $reason }
# $path: the video's file.
record-error-file = The video { $path } couldn't be written: { $reason }

## The control bar on the box's top edge

record-bar-record = Record
record-bar-record-tooltip = Start recording what's in the box
record-bar-pause = Pause
record-bar-pause-tooltip = Pause: nothing is recorded until you resume, and the video has no gap
record-bar-resume = Resume
record-bar-resume-tooltip = Go on recording
record-bar-stop = Stop
record-bar-stop-tooltip = Stop and save the video
record-bar-cancel = Cancel
record-bar-cancel-tooltip = Close the box without recording (Esc)
record-bar-discard = Discard
record-bar-discard-tooltip = Throw this recording away
record-bar-discard-question = Discard this recording?
record-bar-discard-confirm = Discard
record-bar-keep = Keep
# What a screen reader reads for the timer. $time: as the timer shows it ("1:05").
record-bar-elapsed = Recorded { $time }
