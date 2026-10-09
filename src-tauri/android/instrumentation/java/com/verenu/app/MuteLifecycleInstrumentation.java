package com.verenu.app;

import android.Manifest;
import android.app.Activity;
import android.app.Instrumentation;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.media.AudioFormat;
import android.media.AudioManager;
import android.media.AudioTrack;
import android.os.Build;
import android.os.Bundle;
import android.view.View;
import android.view.ViewGroup;
import android.webkit.JavascriptInterface;
import android.webkit.WebView;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import org.json.JSONObject;

/** Opt-in emulator fixture. No keys, accessibility grants, or keyboard changes. */
public final class MuteLifecycleInstrumentation extends Instrumentation {
    private WebView web;
    private AudioManager audio;
    private volatile CountDownLatch reply;
    private volatile String result;

    public final class Replies {
        @JavascriptInterface public void complete(String value) {
            result = value;
            reply.countDown();
        }
    }

    @Override public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override public void onStart() {
        Bundle outcome = new Bundle();
        AudioTrack tone = null;
        int original = -1;
        int status = Activity.RESULT_CANCELED;
        try {
            if (!"ranchu".equals(Build.HARDWARE) && !"goldfish".equals(Build.HARDWARE)) {
                throw new AssertionError("This fixture only runs on an isolated emulator");
            }
            Context context = getTargetContext();
            Intent launch = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
            launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            Activity activity = startActivitySync(launch);
            runOnMainSync(() -> web = findWebView(activity.getWindow().getDecorView()));
            if (web == null) throw new AssertionError("Native WebView missing");
            runOnMainSync(() -> {
                web.addJavascriptInterface(new Replies(), "MuteFixture");
                // Android exposes newly registered JS interfaces after reload.
                web.reload();
            });
            if (activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                runOnMainSync(() -> activity.requestPermissions(new String[]{Manifest.permission.RECORD_AUDIO}, 701));
                long deadline = System.currentTimeMillis() + 60000;
                while (activity.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                    if (System.currentTimeMillis() > deadline) throw new AssertionError("Microphone dialog was not approved");
                    Thread.sleep(100);
                }
            }
            Thread.sleep(2000);
            audio = (AudioManager) context.getSystemService(Context.AUDIO_SERVICE);
            original = audio.getStreamVolume(AudioManager.STREAM_MUSIC);
            short[] samples = new short[16000];
            for (int i = 0; i < samples.length; i++) samples[i] = (short)(1000 * Math.sin(2 * Math.PI * 440 * i / 16000));
            tone = new AudioTrack(AudioManager.STREAM_MUSIC, 16000, AudioFormat.CHANNEL_OUT_MONO,
                AudioFormat.ENCODING_PCM_16BIT, samples.length * 2, AudioTrack.MODE_STATIC);
            tone.write(samples, 0, samples.length);
            tone.setLoopPoints(0, samples.length, -1);
            tone.play();

            invoke("save_setting", "{\"key\":\"mute_audio\",\"value\":false}");
            volume(6);
            invoke("start_input_recording", "{}");
            Thread.sleep(400);
            expectVolume(6);
            invoke("stop_recording", "{}");
            expectMuteReleased();
            expectVolume(6);
            progress("Preference disabled preserves media volume");

            invoke("save_setting", "{\"key\":\"mute_audio\",\"value\":true}");
            for (String terminal : new String[]{"stop_recording", "android_on_permission_revoked"}) {
                volume(6);
                invoke("start_input_recording", "{}");
                expectVolume(0);
                progress("Media muted before " + terminal);
                invoke(terminal, terminal.equals("android_on_permission_revoked") ? "{\"permission\":\"microphone\"}" : "{}");
                expectMuteReleased();
                expectVolume(6);
                progress("Media restored after " + terminal);
            }
            volume(6);
            invoke("start_input_recording", "{}");
            expectVolume(0);
            volume(4);
            Thread.sleep(250);
            volume(0);
            invoke("stop_recording", "{}");
            expectMuteReleased();
            expectVolume(0);
            progress("Observed user override preserved, including a later zero");
            // Android starts its full pipeline asynchronously on normal stop.
            // Test restoration before it completes, with no subsequent session.
            volume(6);
            invoke("start_input_recording", "{}");
            expectVolume(0);
            invoke("stop_handless_mode", "{}");
            expectMuteReleased();
            expectVolume(6);
            outcome.putString("stream", "Mute lifecycle passed: disabled, cancel, normal stop, permission callback, user override");
            status = Activity.RESULT_OK;
        } catch (Throwable error) {
            outcome.putString("stream", "Mute lifecycle failed: " + error.getClass().getSimpleName() + ": " + error.getMessage());
        } finally {
            if (tone != null) { tone.stop(); tone.release(); }
            if (audio != null && original >= 0) volume(original);
        }
        finish(status, outcome);
    }

    private static WebView findWebView(View view) {
        if (view instanceof WebView) return (WebView)view;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup)view;
            for (int i = 0; i < group.getChildCount(); i++) {
                WebView found = findWebView(group.getChildAt(i));
                if (found != null) return found;
            }
        }
        return null;
    }

    private void progress(String message) {
        Bundle update = new Bundle();
        update.putString("stream", message);
        sendStatus(0, update);
    }

    private void invoke(String command, String args) throws Exception {
        reply = new CountDownLatch(1);
        result = null;
        String js = "window.__TAURI_INTERNALS__.invoke(" + JSONObject.quote(command) + "," + args + ")"
            + ".then(()=>MuteFixture.complete('ok'),e=>MuteFixture.complete('error:'+String(e)))";
        runOnMainSync(() -> web.evaluateJavascript(js, null));
        if (!reply.await(30, TimeUnit.SECONDS)) throw new AssertionError("IPC timed out: " + command);
        if (!"ok".equals(result)) throw new AssertionError("IPC failed: " + command + ": " + result);
    }

    private void volume(int level) { audio.setStreamVolume(AudioManager.STREAM_MUSIC, level, 0); }
    private void expectMuteReleased() throws Exception {
        Class<?> bridgeClass = Class.forName("com.verenu.app.VerenuBridge");
        Object bridge = bridgeClass.getConstructor(Context.class).newInstance(getTargetContext());
        long deadline = System.currentTimeMillis() + 3000;
        while (true) {
            Object snapshot = bridgeClass.getMethod("getState").invoke(bridge);
            if (snapshot != null && Boolean.FALSE.equals(snapshot.getClass().getMethod("getAudioMuteRequested").invoke(snapshot))) return;
            if (System.currentTimeMillis() > deadline) throw new AssertionError("Rust mute owner was not released");
            Thread.sleep(25);
        }
    }
    private void expectVolume(int expected) throws Exception {
        long deadline = System.currentTimeMillis() + 3000;
        while (audio.getStreamVolume(AudioManager.STREAM_MUSIC) != expected) {
            if (System.currentTimeMillis() > deadline) throw new AssertionError("Expected media volume " + expected);
            Thread.sleep(25);
        }
    }
}
