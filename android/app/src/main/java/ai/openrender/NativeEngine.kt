package ai.openrender

import org.json.JSONObject

/**
 * Kotlin wrapper around the Rust engine (JNI).
 *
 * Usage:
 * ```
 * val engine = NativeEngine()
 * val applier = DeltaApplier()
 * engine.feed("Hello **wor").let(applier::apply)
 * engine.feed("ld**\n\n").let(applier::apply)
 * engine.finish().let(applier::apply)
 * adapter.submitList(applier.snapshot())
 * ```
 *
 * The JSON deltas are applied on the main thread; parsing itself happens
 * entirely in Rust and is safe to call from any thread.
 */
class NativeEngine(
    throttleMs: Int = 8,
    maxTailBytes: Int = 8 * 1024,
    maxDocumentBytes: Long = 16L * 1024 * 1024,
    enablePreviews: Boolean = true,
) : AutoCloseable {

    private var handle: Long = nativeNew(throttleMs, maxTailBytes, maxDocumentBytes, enablePreviews)

    fun feed(text: String): JSONObject? = decode(nativeFeed(handle, text))

    fun flush(): JSONObject? = decode(nativeFlush(handle))

    fun finish(): JSONObject? = decode(nativeFinish(handle))

    fun reset() {
        if (handle != 0L) nativeReset(handle)
    }

    /** Tune parser features and rendering policy after construction. */
    fun configure(
        parserFlags: Int = PARSE_DEFAULT,
        softBreakAsSpace: Boolean = false,
        dropHtml: Boolean = false,
        sizeHints: Boolean = true,
        maxBlocks: Long = 1_000_000L,
    ) {
        if (handle != 0L) {
            nativeConfigure(handle, parserFlags, softBreakAsSpace, dropHtml, sizeHints, maxBlocks)
        }
    }

    override fun close() {
        if (handle != 0L) {
            nativeFree(handle)
            handle = 0L
        }
    }

    private fun decode(json: String?): JSONObject? =
        json?.takeIf { it.isNotEmpty() }?.let { JSONObject(it) }

    private external fun nativeNew(
        throttleMs: Int,
        maxTailBytes: Int,
        maxDocumentBytes: Long,
        enablePreviews: Boolean,
    ): Long

    private external fun nativeFree(handle: Long)

    private external fun nativeFeed(handle: Long, text: String): String?

    private external fun nativeFlush(handle: Long): String?

    private external fun nativeFinish(handle: Long): String?

    private external fun nativeReset(handle: Long)

    private external fun nativeConfigure(
        handle: Long,
        parserFlags: Int,
        softBreakAsSpace: Boolean,
        dropHtml: Boolean,
        sizeHints: Boolean,
        maxBlocks: Long,
    )

    companion object {
        init {
            System.loadLibrary("openrender")
        }

        const val PARSE_TABLES = 1
        const val PARSE_STRIKETHROUGH = 2
        const val PARSE_TASKLISTS = 4
        const val PARSE_FOOTNOTES = 8
        const val PARSE_GFM = 16
        const val PARSE_MATH = 32
        const val PARSE_SMART_PUNCTUATION = 64

        /** tables | strikethrough | task lists | footnotes | GFM | math */
        const val PARSE_DEFAULT = 0x3F

        /** One-shot rendering without an incremental engine instance.
         *  `@JvmStatic` keeps the JNI symbol as
         *  `Java_ai_openrender_NativeEngine_nativeRender`. */
        @JvmStatic
        external fun nativeRender(text: String): String?

        fun render(text: String): JSONObject? =
            nativeRender(text)?.takeIf { it.isNotEmpty() }?.let { JSONObject(it) }
    }
}

/**
 * Applies delta ops to an ordered block list.
 *
 * `put` replaces/inserts by id, `append` extends a streaming code block,
 * `close` marks it finished, `remove` drops the id (a shrinking tail).
 */
class DeltaApplier {
    private val blocks = LinkedHashMap<Long, JSONObject>()

    var revision: Long = 0
        private set

    fun apply(delta: JSONObject): List<JSONObject> {
        revision = delta.optLong("rev", revision)
        val ops = delta.optJSONArray("ops") ?: return snapshot()
        for (i in 0 until ops.length()) {
            val op = ops.getJSONObject(i)
            when (op.optString("op")) {
                "put" -> {
                    val block = op.getJSONObject("block")
                    blocks[block.optLong("id", -1)] = block
                }
                "append" -> {
                    val id = op.getLong("id")
                    val block = blocks[id] ?: continue
                    block.put("text", block.optString("text") + op.optString("text"))
                }
                "close" -> {
                    blocks[op.getLong("id")]?.put("closed", true)
                }
                "remove" -> blocks.remove(op.getLong("id"))
            }
        }
        return snapshot()
    }

    fun snapshot(): List<JSONObject> =
        blocks.entries.sortedBy { it.key }.map { it.value }

    fun clear() {
        blocks.clear()
        revision = 0
    }
}
