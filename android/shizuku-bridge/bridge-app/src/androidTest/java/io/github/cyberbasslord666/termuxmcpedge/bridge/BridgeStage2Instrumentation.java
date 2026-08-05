package io.github.cyberbasslord666.termuxmcpedge.bridge;

import android.app.Activity;
import android.app.Instrumentation;
import android.os.Bundle;

public final class BridgeStage2Instrumentation extends Instrumentation {
    private static final int EXPECTED_TEST_COUNT = 3;
    private static final String EXACT_TEST_INVENTORY =
            "BridgeManifestInstrumentationTest#testTargetManifestHasNoPermissionOrComponent;"
                    + "BridgeParcelInstrumentationTest#testContextRoundTripPreservesOnlyFixedFields;"
                    + "BridgeParcelInstrumentationTest#"
                    + "testUnknownContextVersionFailsClosedDuringUnparcel";

    @Override
    public void onCreate(Bundle arguments) {
        super.onCreate(arguments);
        start();
    }

    @Override
    public void onStart() {
        Bundle result = new Bundle();
        int completed = 0;
        try {
            BridgeManifestInstrumentationTest manifestTest =
                    new BridgeManifestInstrumentationTest();
            manifestTest.testTargetManifestHasNoPermissionOrComponent(getTargetContext());
            completed++;

            BridgeParcelInstrumentationTest parcelTest =
                    new BridgeParcelInstrumentationTest();
            parcelTest.testContextRoundTripPreservesOnlyFixedFields();
            completed++;
            parcelTest.testUnknownContextVersionFailsClosedDuringUnparcel();
            completed++;

            if (completed != EXPECTED_TEST_COUNT) {
                throw new AssertionError("instrumentation count mismatch");
            }
            result.putInt("numtests", EXPECTED_TEST_COUNT);
            result.putString("tests", EXACT_TEST_INVENTORY);
            result.putString("stream", "\nOK (3 tests)\n");
            finish(Activity.RESULT_OK, result);
        } catch (Throwable failure) {
            result.putInt("numtests", EXPECTED_TEST_COUNT);
            result.putInt("completed", completed);
            result.putString("stream", "\nFAILURES (completed=" + completed + ")\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
