# AndroidX security-crypto references these optional annotation types. They
# are compile-time metadata only and are not required at runtime.
-dontwarn javax.annotation.Nullable
-dontwarn javax.annotation.concurrent.GuardedBy

# Tauri discovers plugin commands and callbacks by reflection.
-keep class com.verenu.app.VerenuPermissionPlugin { *; }
-keep class com.verenu.app.VerenuSecurityPlugin { *; }
