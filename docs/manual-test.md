# Windows manual checks

1. Launch on a machine without virtual audio software. The English main window fits without scrolling; System limiter is usable after selecting a physical output.
2. Select Night / Headphones / Speakers and adjust the peak target. Start and stop native processing. Confirm real meter readings and the starting device volume is restored after Stop.
3. During attenuation, change volume using the keyboard or Windows mixer. Confirm the new user level is respected and Stop does not restore above it. Repeat with mute enabled; the app must not unmute it.
4. Hide the window, choose **Open Sound Lock** from the tray, then repeat using a left click. Repeat after minimizing and after closing with X. Verify the native worker continues while hidden and Quit stops/restores it.
5. Open Setup. Device changes are disabled during processing. Disconnect the selected endpoint, reconnect it, refresh and restart. It must not silently switch to another output or a microphone.
6. For PCM bridge tests, choose a genuinely inaudible existing source endpoint and a different physical listening endpoint. Route only the test player to the source. Do not install a virtual driver for this test.
7. Compare quiet speech and loud scenes at 100–400% with leveling on/off. Verify stereo balance and absence of double playback. The source endpoint must stay inaudible independently of the app.
8. Check 44.1/48 kHz and matching 48 kHz, at least 30 minutes of playback, Bluetooth reconnects and audiovisual latency. Record OS/device details and CPU use.
9. Quit normally, restore the player's usual routing, and relaunch. Settings persist; processing does not start itself.
10. Open Setup and both tabs. The background must be dimmed; clicking or dragging background controls must have no effect and must not dismiss the modal. Close/Escape restore input, and Escape closes an open combo before the modal.
11. On a machine where driver installation is explicitly wanted, open Driver (optional). Download must leave the system unchanged and enable Install driver. Cancel Windows UAC and confirm the app remains responsive and can retry. Install through the vendor UI only when intended, restart Windows, Refresh devices and Use VB-CABLE. Select CABLE Input as the browser output and verify full DSP with a single physical listening device. Confirm an already detected cable disables installation. These installation steps are manual and are never run by automated tests.

Automated DSP checks do not validate subjective speech clarity or every audio driver. Consult `validation.md` for checks actually completed in this session.
