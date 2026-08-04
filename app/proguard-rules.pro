# ProGuard rules for Sakichan SE release builds

# kotlinx.serialization
-keepattributes *Annotation*, InnerClasses
-dontnote kotlinx.serialization.AnnotationsKt
-keepclassmembers class kotlinx.serialization.json.** {
    *** Companion;
}
-keepclasseswithmembers class kotlinx.serialization.json.** {
    kotlinx.serialization.KSerializer serializer(...);
}
-keep,includedescriptorclasses class com.sakichan.se.**$$serializer { *; }
-keepclassmembers class com.sakichan.se.** {
    *** Companion;
}
-keepclasseswithmembers class com.sakichan.se.** {
    kotlinx.serialization.KSerializer serializer(...);
}

# OkHttp
-dontwarn okhttp3.**
-dontwarn okio.**

# Koin
-keep class org.koin.** { *; }
