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
            String failureCheckpoint;
            if (failure instanceof BridgeParcelInstrumentationTest.CheckpointFailure) {
                failureCheckpoint =
                        ((BridgeParcelInstrumentationTest.CheckpointFailure) failure)
                                .getCheckpoint();
            } else if (completed == 0) {
                failureCheckpoint = "M00";
            } else if (completed == 1) {
                failureCheckpoint = "R00";
            } else {
                failureCheckpoint = "P00";
            }
            result.putInt("numtests", EXPECTED_TEST_COUNT);
            result.putInt("completed", completed);
            result.putString("failure", failureCheckpoint);
            result.putString("stream", "\nFAILURES (checkpoint=" + failureCheckpoint + ")\n");
            finish(Activity.RESULT_CANCELED, result);
        }
    }
}
