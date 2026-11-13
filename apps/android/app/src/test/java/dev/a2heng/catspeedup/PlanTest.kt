package dev.a2heng.catspeedup

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PlanTest {
    private fun span(startMs: Long, endMs: Long) = SilenceSpan(startMs, endMs)

    @Test
    fun `no silence keeps one voice segment`() {
        val plan = SilenceDetector.buildPlan(10_000, emptyList(), PlanOptions())
        assertEquals(1, plan.segments.size)
        assertTrue(plan.segments[0].isVoice)
        assertEquals(10_000, plan.outputDurationMs())
    }

    @Test
    fun `middle silence keeps order and speeds only the gap`() {
        val plan = SilenceDetector.buildPlan(
            10_000,
            listOf(span(4_000, 6_000)),
            PlanOptions(marginMs = 0, silenceSpeed = 4f),
        )
        assertEquals(3, plan.segments.size)
        assertTrue(plan.segments[0].isVoice)
        assertTrue(!plan.segments[1].isVoice)
        assertTrue(plan.segments[2].isVoice)
        assertEquals(4_000, plan.segments[1].startMs)
        assertEquals(6_000, plan.segments[1].endMs)
        assertEquals(4_000 + 500 + 4_000, plan.outputDurationMs())
    }

    @Test
    fun `margin expands voice without crossing the duration`() {
        val plan = SilenceDetector.buildPlan(
            10_000,
            listOf(span(0, 2_000), span(8_000, 10_000)),
            PlanOptions(marginMs = 500, silenceSpeed = 2f),
        )
        assertEquals(1_500, plan.segments[0].endMs)
        assertEquals(1_500, plan.segments[1].startMs)
        assertEquals(8_500, plan.segments[1].endMs)
        assertEquals(8_500, plan.segments[2].startMs)
    }

    @Test
    fun `short silence is absorbed by margin`() {
        val plan = SilenceDetector.buildPlan(
            10_000,
            listOf(span(4_000, 4_200)),
            PlanOptions(marginMs = 150),
        )
        assertEquals(1, plan.segments.size)
        assertTrue(plan.segments[0].isVoice)
    }

    @Test
    fun `overlapping silences are merged`() {
        val plan = SilenceDetector.buildPlan(
            10_000,
            listOf(span(2_000, 5_000), span(4_000, 6_000)),
            PlanOptions(marginMs = 0),
        )
        assertEquals(3, plan.segments.size)
        assertEquals(2_000, plan.segments[1].startMs)
        assertEquals(6_000, plan.segments[1].endMs)
    }

    @Test
    fun `plan is gap free and covers the whole source`() {
        val plan = SilenceDetector.buildPlan(
            60_000,
            listOf(span(0, 1_000), span(3_000, 4_000), span(20_000, 21_000), span(59_000, 60_000)),
            PlanOptions(),
        )
        assertEquals(0, plan.segments.first().startMs)
        assertEquals(60_000, plan.segments.last().endMs)
        plan.segments.zipWithNext().forEach { (left, right) ->
            assertEquals(left.endMs, right.startMs)
        }
    }

    @Test
    fun `segment lookup finds the segment at a position`() {
        val plan = SilenceDetector.buildPlan(
            10_000,
            listOf(span(4_000, 6_000)),
            PlanOptions(marginMs = 0),
        )
        assertEquals(4.0f, plan.segmentAt(4_500)?.speed ?: 0f, 0.001f)
        assertEquals(1.0f, plan.segmentAt(6_500)?.speed ?: 0f, 0.001f)
        assertNull(plan.segmentAt(10_000))
    }

    @Test
    fun `srt uses standard timestamps and numbering`() {
        val srt = toSrt(
            listOf(
                Subtitle(0, 1_500, "第一句"),
                Subtitle(3_723, 61_004, "第二句"),
            ),
        )
        assertEquals(
            "1\n00:00:00,000 --> 00:00:01,500\n第一句\n\n" +
                "2\n00:00:03,723 --> 00:01:01,004\n第二句\n\n",
            srt,
        )
    }
}
