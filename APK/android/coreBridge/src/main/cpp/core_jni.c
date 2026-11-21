#include <jni.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>
#include "libchaindesk_core.h"

static JavaVM *java_vm;
static pthread_mutex_t session_mutex = PTHREAD_MUTEX_INITIALIZER;
static jobject platform_callback;
static jmethodID protect_method;
static jlong network_handle;
static jlong session_id;

JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void *reserved) {
    (void)reserved;
    java_vm = vm;
    return JNI_VERSION_1_6;
}

static int protect_socket(int fd) {
    JNIEnv *env = NULL;
    int attached = 0;
    jint status = (*java_vm)->GetEnv(java_vm, (void **)&env, JNI_VERSION_1_6);
    if (status == JNI_EDETACHED) {
        if ((*java_vm)->AttachCurrentThread(java_vm, &env, NULL) != JNI_OK) return 0;
        attached = 1;
    } else if (status != JNI_OK) {
        return 0;
    }
    jboolean allowed = (*env)->CallBooleanMethod(env, platform_callback, protect_method, (jint)fd, network_handle);
    if ((*env)->ExceptionCheck(env)) {
        (*env)->ExceptionClear(env);
        allowed = JNI_FALSE;
    }
    if (attached) (*java_vm)->DetachCurrentThread(java_vm);
    return allowed == JNI_TRUE;
}

JNIEXPORT jstring JNICALL
Java_dev_chaindesk_core_NativeBridge_selfCheck(JNIEnv *env, jobject instance) {
    (void)instance;
    char *result = ChainDeskSelfCheck();
    jstring value = (*env)->NewStringUTF(env, result);
    ChainDeskFree(result);
    return value;
}

JNIEXPORT jboolean JNICALL
Java_dev_chaindesk_core_NativeBridge_validateFixture(JNIEnv *env, jobject instance, jbyteArray data) {
    (void)instance;
    if (data == NULL) return JNI_FALSE;
    jsize length = (*env)->GetArrayLength(env, data);
    if (length <= 0 || length > 16384) return JNI_FALSE;
    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) return JNI_FALSE;
    int valid = ChainDeskValidateFixture((char *)bytes, length);
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
    return valid ? JNI_TRUE : JNI_FALSE;
}

static jlong start_core(JNIEnv *env, jbyteArray data, jint borrowed_fd, jlong network, jobject callbacks, int product) {
    if (data == NULL || callbacks == NULL || borrowed_fd < 0 || network == 0) return 0;
    jsize length = (*env)->GetArrayLength(env, data);
    if (length <= 0 || length > (product ? 131072 : 16384)) return 0;
    pthread_mutex_lock(&session_mutex);
    if (platform_callback != NULL) {
        pthread_mutex_unlock(&session_mutex);
        return 0;
    }
    jclass callback_class = (*env)->GetObjectClass(env, callbacks);
    if (callback_class == NULL) {
        pthread_mutex_unlock(&session_mutex);
        return 0;
    }
    protect_method = (*env)->GetMethodID(env, callback_class, "protectSocket", "(IJ)Z");
    (*env)->DeleteLocalRef(env, callback_class);
    if (protect_method == NULL) {
        pthread_mutex_unlock(&session_mutex);
        return 0;
    }
    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) {
        pthread_mutex_unlock(&session_mutex);
        return 0;
    }
    platform_callback = (*env)->NewGlobalRef(env, callbacks);
    int owned_fd = platform_callback == NULL ? -1 : fcntl(borrowed_fd, F_DUPFD_CLOEXEC, 3);
    network_handle = network;
    // Go consumes owned_fd even when its start operation fails.
    session_id = owned_fd < 0 ? 0 : product
        ? (jlong)ChainDeskStartChain((char *)bytes, length, owned_fd, (uintptr_t)protect_socket)
        : (jlong)ChainDeskStart((char *)bytes, length, owned_fd, (uintptr_t)protect_socket);
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
    if (session_id == 0 && platform_callback != NULL) {
        (*env)->DeleteGlobalRef(env, platform_callback);
        platform_callback = NULL;
    }
    jlong result = session_id;
    pthread_mutex_unlock(&session_mutex);
    return result;
}

JNIEXPORT jlong JNICALL
Java_dev_chaindesk_core_NativeBridge_start(JNIEnv *env, jobject instance, jbyteArray data, jint borrowed_fd, jlong network, jobject callbacks) {
    (void)instance;
    return start_core(env, data, borrowed_fd, network, callbacks, 0);
}

JNIEXPORT jlong JNICALL
Java_dev_chaindesk_core_NativeBridge_startChain(JNIEnv *env, jobject instance, jbyteArray data, jint borrowed_fd, jlong network, jobject callbacks) {
    (void)instance;
    return start_core(env, data, borrowed_fd, network, callbacks, 1);
}

JNIEXPORT jboolean JNICALL
Java_dev_chaindesk_core_NativeBridge_stop(JNIEnv *env, jobject instance, jlong id) {
    (void)instance;
    jboolean stopped = JNI_TRUE;
    pthread_mutex_lock(&session_mutex);
    if (id != 0 && id == session_id) {
        // Block waits for every protect callback before its global ref is freed.
        stopped = ChainDeskStop((unsigned long long)id) ? JNI_TRUE : JNI_FALSE;
        (*env)->DeleteGlobalRef(env, platform_callback);
        platform_callback = NULL;
        session_id = 0;
    }
    pthread_mutex_unlock(&session_mutex);
    return stopped;
}

JNIEXPORT void JNICALL
Java_dev_chaindesk_core_NativeBridge_blockSockets(JNIEnv *env, jobject instance, jlong id) {
    (void)env;
    (void)instance;
    ChainDeskBlockSockets((unsigned long long)id);
}

JNIEXPORT void JNICALL
Java_dev_chaindesk_core_NativeBridge_cancelProbe(JNIEnv *env, jobject instance, jlong id) {
    (void)env;
    (void)instance;
    ChainDeskCancelProbe((unsigned long long)id);
}

JNIEXPORT jboolean JNICALL
Java_dev_chaindesk_core_NativeBridge_activate(JNIEnv *env, jobject instance, jlong id) {
    (void)env;
    (void)instance;
    return ChainDeskActivate((unsigned long long)id) ? JNI_TRUE : JNI_FALSE;
}

static jbyteArray copy_json(JNIEnv *env, const char *json) {
    size_t length = strlen(json);
    if (length > 67108864) return NULL;
    jbyteArray result = (*env)->NewByteArray(env, (jsize)length);
    if (result != NULL) (*env)->SetByteArrayRegion(env, result, 0, (jsize)length, (const jbyte *)json);
    return result;
}

static jbyteArray bytes_operation(JNIEnv *env, jbyteArray data, int maximum, char *(*operation)(char *, int)) {
    if (data == NULL) return copy_json(env, "{\"errorCode\":\"BRIDGE_INPUT_INVALID\"}");
    jsize length = (*env)->GetArrayLength(env, data);
    if (length <= 0 || length > maximum) return copy_json(env, "{\"errorCode\":\"BRIDGE_INPUT_INVALID\"}");
    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) return NULL;
    char *json = operation((char *)bytes, length);
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
    if (json == NULL) return NULL;
    jbyteArray result = copy_json(env, json);
    ChainDeskFree(json);
    return result;
}

JNIEXPORT jbyteArray JNICALL
Java_dev_chaindesk_core_NativeBridge_importSubscription(JNIEnv *env, jobject instance, jbyteArray data) {
    (void)instance;
    return bytes_operation(env, data, 10485760, ChainDeskImportSubscription);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_chaindesk_core_NativeBridge_compileChain(JNIEnv *env, jobject instance, jbyteArray data) {
    (void)instance;
    return bytes_operation(env, data, 131072, ChainDeskCompileChain);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_chaindesk_core_NativeBridge_ruleManifest(JNIEnv *env, jobject instance) {
    (void)instance;
    char *json = ChainDeskRuleManifest();
    if (json == NULL) return NULL;
    jbyteArray result = copy_json(env, json);
    ChainDeskFree(json);
    return result;
}

JNIEXPORT jbyteArray JNICALL
Java_dev_chaindesk_core_NativeBridge_setRulesDirectory(JNIEnv *env, jobject instance, jbyteArray data) {
    (void)instance;
    return bytes_operation(env, data, 4096, ChainDeskSetRulesDirectory);
}

JNIEXPORT jbyteArray JNICALL
Java_dev_chaindesk_core_NativeBridge_probe(JNIEnv *env, jobject instance, jlong id, jbyteArray data) {
    (void)instance;
    if (data == NULL) return copy_json(env, "{\"errorCode\":\"PROBE_INPUT_INVALID\"}");
    jsize length = (*env)->GetArrayLength(env, data);
    if (length <= 0 || length > 4096) return copy_json(env, "{\"errorCode\":\"PROBE_INPUT_INVALID\"}");
    jbyte *bytes = (*env)->GetByteArrayElements(env, data, NULL);
    if (bytes == NULL) return NULL;
    char *json = ChainDeskProbe((unsigned long long)id, (char *)bytes, length);
    (*env)->ReleaseByteArrayElements(env, data, bytes, JNI_ABORT);
    if (json == NULL) return NULL;
    jbyteArray result = copy_json(env, json);
    ChainDeskFree(json);
    return result;
}
