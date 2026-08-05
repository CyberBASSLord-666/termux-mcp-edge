package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.os.Parcel;

import java.util.Arrays;
import java.util.Objects;

final class BridgeParcelCodec {
    static final int SHA256_BYTES = 32;
    static final int NONCE_BYTES = 32;
    static final int MAX_MANAGER_VERSION_NAME_BYTES = 64;
    static final int MAX_PARCELABLE_ARRAY_ELEMENTS = 16;
    static final int MAX_FRAME_BYTES = 4096;

    private static final int INT_BYTES = Integer.BYTES;
    private static final int LONG_BYTES = Long.BYTES;
    private static final int PARCEL_ALIGNMENT_BYTES = Integer.BYTES;
    private static final int MIN_FRAME_BYTES = INT_BYTES * 2;

    private BridgeParcelCodec() {
        throw new AssertionError("No instances");
    }

    static void requireProtocolVersion(int version) {
        if (version != BridgeCallContext.PROTOCOL_VERSION) {
            throw new IllegalArgumentException("unsupported protocol version");
        }
    }

    static void requireParcelVersion(int version, int expectedVersion) {
        if (version != expectedVersion) {
            throw new IllegalArgumentException("unsupported parcel version");
        }
    }

    static long requireRequestId(long requestId) {
        if (requestId <= 0L) {
            throw new IllegalArgumentException("request id must be positive");
        }
        return requestId;
    }

    static byte[] requireFixed(byte[] value, int expectedLength) {
        if (value == null || value.length != expectedLength) {
            throw new IllegalArgumentException("invalid fixed byte length");
        }
        return Arrays.copyOf(value, value.length);
    }

    static byte[] copy(byte[] value) {
        return Arrays.copyOf(value, value.length);
    }

    static int requireArraySize(int size) {
        if (size < 0 || size > MAX_PARCELABLE_ARRAY_ELEMENTS) {
            throw new IllegalArgumentException("invalid parcelable array size");
        }
        return size;
    }

    static String requireAsciiToken(String value, int maximumBytes) {
        if (value == null) {
            throw new IllegalArgumentException("missing ASCII token");
        }
        int length = value.length();
        if (length == 0 || length > maximumBytes) {
            throw new IllegalArgumentException("invalid ASCII token length");
        }
        for (int index = 0; index < length; index++) {
            char character = value.charAt(index);
            boolean accepted = character >= 'a' && character <= 'z'
                    || character >= 'A' && character <= 'Z'
                    || character >= '0' && character <= '9'
                    || character == '.'
                    || character == '_'
                    || character == '+'
                    || character == '-';
            if (!accepted) {
                throw new IllegalArgumentException("invalid ASCII token");
            }
        }
        return value;
    }

    static int beginWriteFrame(Parcel destination) {
        Objects.requireNonNull(destination, "destination");
        int startPosition = destination.dataPosition();
        destination.writeInt(0);
        return startPosition;
    }

    static void finishWriteFrame(Parcel destination, int startPosition) {
        Objects.requireNonNull(destination, "destination");
        int endPosition = destination.dataPosition();
        long encodedSize = (long) endPosition - startPosition;
        requireValidFrameSize(encodedSize);

        destination.setDataPosition(startPosition);
        destination.writeInt((int) encodedSize);
        if (destination.dataPosition() != startPosition + INT_BYTES) {
            throw new IllegalStateException("failed to write parcel frame size");
        }
        destination.setDataPosition(endPosition);
        if (destination.dataPosition() != endPosition) {
            throw new IllegalStateException("failed to restore parcel write position");
        }
    }

    static ReadFrame beginReadFrame(Parcel source) {
        Objects.requireNonNull(source, "source");
        return beginReadFrame(source, source.dataSize());
    }

    static ReadFrame beginNestedReadFrame(Parcel source, ReadFrame parent) {
        Objects.requireNonNull(parent, "parent");
        return beginReadFrame(source, parent.endPosition);
    }

    private static ReadFrame beginReadFrame(Parcel source, int outerEndPosition) {
        Objects.requireNonNull(source, "source");
        int startPosition = source.dataPosition();
        int dataSize = source.dataSize();
        if (startPosition < 0
                || outerEndPosition < startPosition
                || outerEndPosition > dataSize) {
            throw new IllegalArgumentException("invalid enclosing parcel frame");
        }
        requireRemaining(source, outerEndPosition, INT_BYTES);
        int headerPosition = source.dataPosition();
        int encodedSize = source.readInt();
        requireExactAdvance(source, headerPosition, INT_BYTES);
        requireValidFrameSize(encodedSize);
        long endPosition = (long) startPosition + encodedSize;
        if (endPosition > outerEndPosition || endPosition > dataSize) {
            throw new IllegalArgumentException("parcel frame exceeds enclosing data");
        }
        return new ReadFrame((int) endPosition);
    }

    static void finishReadFrame(Parcel source, ReadFrame frame) {
        Objects.requireNonNull(source, "source");
        Objects.requireNonNull(frame, "frame");
        if (source.dataPosition() != frame.endPosition) {
            throw new IllegalArgumentException("parcel frame has missing or trailing fields");
        }
    }

    static int readInt(Parcel source, ReadFrame frame) {
        requireRemaining(source, frame.endPosition, INT_BYTES);
        int startPosition = source.dataPosition();
        int value = source.readInt();
        requireExactAdvance(source, startPosition, INT_BYTES);
        return value;
    }

    static long readLong(Parcel source, ReadFrame frame) {
        requireRemaining(source, frame.endPosition, LONG_BYTES);
        int startPosition = source.dataPosition();
        long value = source.readLong();
        requireExactAdvance(source, startPosition, LONG_BYTES);
        return value;
    }

    static boolean readBoolean(Parcel source, ReadFrame frame) {
        int encoded = readInt(source, frame);
        if (encoded != 0 && encoded != 1) {
            throw new IllegalArgumentException("invalid encoded boolean");
        }
        return encoded == 1;
    }

    static void writeFixed(Parcel destination, byte[] value) {
        Objects.requireNonNull(destination, "destination");
        if (value == null || value.length != SHA256_BYTES) {
            throw new IllegalArgumentException("invalid fixed byte length");
        }
        destination.writeByteArray(value);
    }

    static byte[] readFixed(Parcel source, ReadFrame frame, int expectedLength) {
        if (expectedLength < 0 || expectedLength > SHA256_BYTES) {
            throw new IllegalArgumentException("invalid fixed byte length");
        }
        int encodedBytes = INT_BYTES + alignedBytes(expectedLength);
        requireRemaining(source, frame.endPosition, encodedBytes);
        int startPosition = source.dataPosition();
        byte[] result = new byte[expectedLength];
        try {
            source.readByteArray(result);
        } catch (RuntimeException ignored) {
            throw new IllegalArgumentException("invalid fixed byte array encoding");
        }
        requireExactAdvance(source, startPosition, encodedBytes);
        return result;
    }

    static void writeAsciiToken(Parcel destination, String value) {
        destination.writeInt(value.length());
        for (int index = 0; index < value.length(); index++) {
            destination.writeInt(value.charAt(index));
        }
    }

    static String readAsciiToken(Parcel source, ReadFrame frame, int maximumBytes) {
        int length = readInt(source, frame);
        if (length <= 0 || length > maximumBytes) {
            throw new IllegalArgumentException("invalid encoded token length");
        }
        requireRemaining(source, frame.endPosition, Math.multiplyExact(length, INT_BYTES));
        char[] characters = new char[length];
        for (int index = 0; index < length; index++) {
            int value = readInt(source, frame);
            if (value < 0 || value > 0x7f) {
                throw new IllegalArgumentException("non-ASCII encoded token");
            }
            characters[index] = (char) value;
        }
        return requireAsciiToken(new String(characters), maximumBytes);
    }

    private static void requireValidFrameSize(long encodedSize) {
        if (encodedSize < MIN_FRAME_BYTES
                || encodedSize > MAX_FRAME_BYTES
                || encodedSize % PARCEL_ALIGNMENT_BYTES != 0L) {
            throw new IllegalArgumentException("invalid parcel frame size");
        }
    }

    private static void requireRemaining(
            Parcel source, int endPosition, int requiredBytes) {
        Objects.requireNonNull(source, "source");
        int position = source.dataPosition();
        if (requiredBytes < 0
                || position < 0
                || position > endPosition
                || (long) position + requiredBytes > endPosition
                || (long) position + requiredBytes > source.dataSize()) {
            throw new IllegalArgumentException("truncated parcel frame");
        }
    }

    private static void requireExactAdvance(
            Parcel source, int startPosition, int expectedBytes) {
        if (source.dataPosition() != startPosition + expectedBytes) {
            throw new IllegalArgumentException("unexpected parcel cursor advance");
        }
    }

    private static int alignedBytes(int byteCount) {
        return Math.addExact(byteCount, PARCEL_ALIGNMENT_BYTES - 1)
                / PARCEL_ALIGNMENT_BYTES
                * PARCEL_ALIGNMENT_BYTES;
    }

    static final class ReadFrame {
        private final int endPosition;

        private ReadFrame(int endPosition) {
            this.endPosition = endPosition;
        }
    }
}
