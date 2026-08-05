package io.github.cyberbasslord666.termuxmcpedge.bridge;

final class BridgeTestAssertions {
    private BridgeTestAssertions() {
        throw new AssertionError("No instances");
    }

    static void isNull(Object value) {
        if (value != null) {
            throw new AssertionError("expected null");
        }
    }

    static void isTrue(boolean value) {
        if (!value) {
            throw new AssertionError("expected true");
        }
    }

    static void isFalse(boolean value) {
        if (value) {
            throw new AssertionError("expected false");
        }
    }

    static void equalsInt(int expected, int actual) {
        if (expected != actual) {
            throw new AssertionError("integer mismatch");
        }
    }

    static void equalsLong(long expected, long actual) {
        if (expected != actual) {
            throw new AssertionError("long mismatch");
        }
    }

    static void equalsString(String expected, String actual) {
        if (!expected.equals(actual)) {
            throw new AssertionError("string mismatch");
        }
    }
}
