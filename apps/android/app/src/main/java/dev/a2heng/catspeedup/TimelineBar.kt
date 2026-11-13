package dev.a2heng.catspeedup

import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.RectF
import android.util.AttributeSet
import android.view.MotionEvent
import android.view.View
import androidx.core.content.ContextCompat

/**
 * Source-timeline progress bar: the track is split by the playback plan so
 * voice (1x) and accelerated gaps have different colors. Dragging seeks.
 */
class TimelineBar @JvmOverloads constructor(
    context: Context,
    attrs: AttributeSet? = null,
    defStyleAttr: Int = 0,
) : View(context, attrs, defStyleAttr) {
    var plan: PlaybackPlan? = null
        set(value) {
            field = value
            invalidate()
        }

    /** Source position in ms. Ignored while the user drags. */
    var positionMs: Long = 0
        set(value) {
            if (isSeeking) return
            field = value
            invalidate()
        }

    var durationMs: Long = 0
        set(value) {
            field = value
            invalidate()
        }

    var onSeek: ((Long) -> Unit)? = null
    var onSeekingChanged: ((Boolean) -> Unit)? = null

    var isSeeking: Boolean = false
        private set

    private val density = resources.displayMetrics.density
    private val barHeight = dp(5f)
    private val barRadius = dp(2f)

    private val paint = Paint(Paint.ANTI_ALIAS_FLAG)
    private val trackRect = RectF()
    private val playedColor = ContextCompat.getColor(context, R.color.timeline_played)
    private val remainingColor = ContextCompat.getColor(context, R.color.timeline_remaining)

    init {
        isClickable = true
        isFocusable = true
        contentDescription = context.getString(R.string.timeline_desc)
    }

    override fun onDraw(canvas: Canvas) {
        val total = (plan?.sourceDurationMs ?: durationMs).takeIf { it > 0 } ?: return
        val left = paddingLeft.toFloat()
        val right = (width - paddingRight).toFloat()
        if (right <= left) return
        val top = height / 2f - barHeight / 2f
        trackRect.set(left, top, right, top + barHeight)

        // Unplayed: dim, played: bright. Plain rectangle, no thumb.
        paint.color = remainingColor
        canvas.drawRoundRect(trackRect, barRadius, barRadius, paint)

        val x = left + (right - left) * positionMs.coerceIn(0, total) / total.toFloat()
        if (x > left) {
            canvas.save()
            canvas.clipRect(left, top, x, top + barHeight)
            paint.color = playedColor
            canvas.drawRoundRect(trackRect, barRadius, barRadius, paint)
            canvas.restore()
        }
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                isSeeking = true
                parent?.requestDisallowInterceptTouchEvent(true)
                onSeekingChanged?.invoke(true)
                seekTo(event.x)
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                if (isSeeking) {
                    seekTo(event.x)
                    return true
                }
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (isSeeking) {
                    seekTo(event.x)
                    isSeeking = false
                    parent?.requestDisallowInterceptTouchEvent(false)
                    onSeekingChanged?.invoke(false)
                    performClick()
                    return true
                }
            }
        }
        return super.onTouchEvent(event)
    }

    override fun performClick(): Boolean {
        super.performClick()
        return true
    }

    private fun seekTo(x: Float) {
        val total = (plan?.sourceDurationMs ?: durationMs).takeIf { it > 0 } ?: return
        val left = paddingLeft.toFloat()
        val right = (width - paddingRight).toFloat()
        if (right <= left) return
        val fraction = ((x - left) / (right - left)).coerceIn(0f, 1f)
        val target = (total * fraction).toLong()
        positionMs = target
        invalidate()
        onSeek?.invoke(target)
    }

    private fun dp(value: Float): Float = value * density
}
