# AndroidX security-crypto references these optional annotation types. They
# are compile-time metadata only and are not required at runtime.
-dontwarn javax.annotation.Nullable
-dontwarn javax.annotation.concurrent.GuardedBy

# Tauri discovers plugin commands and callbacks by reflection.
-keep class com.verenu.app.VerenuPermissionPlugin { *; }
-keep class com.verenu.app.VerenuSecurityPlugin { *; }
-keep class com.verenu.app.VerenuUpdaterPlugin { *; }

# Rust calls the Application directly through JNI, outside Tauri plugins.
-keepclassmembers class com.verenu.app.VerenuApplication {
    public void updateMediaMute(boolean);
}
