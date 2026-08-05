package io.github.cyberbasslord666.termuxmcpedge.bridge;

/**
 * Compile-time marker for the inert Stage-2 artifact.
 *
 * <p>This class deliberately owns no Android component and exposes no runtime authority.</p>
 */
public final class BridgeSkeleton {
    public static final int SKELETON_STAGE = 2;
    public static final boolean RUNTIME_AUTHORITY_ENABLED = false;
    public static final boolean SHIZUKU_LINKED = false;

    private BridgeSkeleton() {
        throw new AssertionError("No instances");
    }
}
