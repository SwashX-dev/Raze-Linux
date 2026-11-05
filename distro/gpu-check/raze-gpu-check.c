/*
 * raze-gpu-check: exit 0 if a DRM render node provides a hardware-accelerated
 * OpenGL ES renderer, 1 otherwise. niri refuses software renderers (llvmpipe,
 * softpipe, swrast); raze-session uses this to pick niri or the software session.
 * `-v` prints the renderer found on each node.
 *
 * Replaces `eglinfo` from mesa-demos (61 MiB) with a ~16 KiB binary.
 */
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <dirent.h>
#include <fcntl.h>
#include <gbm.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

static int is_software(const char *r)
{
    return !r || strstr(r, "llvmpipe") || strstr(r, "softpipe") || strstr(r, "swrast") ||
           strstr(r, "Software");
}

static int probe(const char *path, int verbose)
{
    int accelerated = 0;
    int fd = open(path, O_RDWR | O_CLOEXEC);
    if (fd < 0)
        return 0;
    struct gbm_device *gbm = gbm_create_device(fd);
    if (!gbm) {
        close(fd);
        return 0;
    }
    PFNEGLGETPLATFORMDISPLAYEXTPROC get_display =
        (PFNEGLGETPLATFORMDISPLAYEXTPROC)eglGetProcAddress("eglGetPlatformDisplayEXT");
    EGLDisplay dpy = get_display ? get_display(EGL_PLATFORM_GBM_KHR, gbm, NULL) : EGL_NO_DISPLAY;
    if (dpy != EGL_NO_DISPLAY && eglInitialize(dpy, NULL, NULL)) {
        eglBindAPI(EGL_OPENGL_ES_API);
        const EGLint attrs[] = {EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE};
        /* EGL_KHR_no_config_context + EGL_KHR_surfaceless_context (Mesa). */
        EGLContext ctx = eglCreateContext(dpy, (EGLConfig)0, EGL_NO_CONTEXT, attrs);
        if (ctx != EGL_NO_CONTEXT) {
            if (eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)) {
                const char *r = (const char *)glGetString(GL_RENDERER);
                if (verbose)
                    printf("%s: %s\n", path, r ? r : "(no renderer)");
                accelerated = !is_software(r);
                eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, EGL_NO_CONTEXT);
            }
            eglDestroyContext(dpy, ctx);
        }
        eglTerminate(dpy);
    } else if (verbose) {
        printf("%s: EGL initialisation failed\n", path);
    }
    gbm_device_destroy(gbm);
    close(fd);
    return accelerated;
}

int main(int argc, char **argv)
{
    int verbose = argc > 1 && strcmp(argv[1], "-v") == 0;
    DIR *d = opendir("/dev/dri");
    if (!d) {
        if (verbose)
            puts("no /dev/dri: no GPU");
        return 1;
    }
    int accelerated = 0;
    struct dirent *e;
    while (!accelerated && (e = readdir(d))) {
        if (strncmp(e->d_name, "renderD", 7) != 0)
            continue;
        char path[288];
        snprintf(path, sizeof path, "/dev/dri/%s", e->d_name);
        accelerated = probe(path, verbose);
    }
    closedir(d);
    return accelerated ? 0 : 1;
}
