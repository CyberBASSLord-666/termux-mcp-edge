package io.github.cyberbasslord666.termuxmcpedge.bridge;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;

import org.junit.Test;

public final class BridgeSkeletonTest {
    @Test
    public void stageTwoArtifactHasNoRuntimeAuthority() {
        assertEquals(2, BridgeSkeleton.SKELETON_STAGE);
        assertFalse(BridgeSkeleton.RUNTIME_AUTHORITY_ENABLED);
        assertFalse(BridgeSkeleton.SHIZUKU_LINKED);
    }
}
