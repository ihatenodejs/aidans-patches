package app.aidan.patches.shared

import app.morphe.patcher.patch.BytecodePatch
import app.morphe.patcher.patch.BytecodePatchContext
import app.morphe.patcher.patch.Compatibility
import app.morphe.patcher.patch.Option
import app.morphe.patcher.patch.Patch
import app.morphe.patcher.patch.ResourcePatchContext
import app.morphe.patcher.patch.bytecodePatch
import app.morphe.patcher.patch.rawResourcePatch
import app.morphe.patcher.patch.resourcePatch

/**
 * Builder for declaring 1-to-1 composite patches that combine Dalvik bytecode mutations
 * with an internal companion resource or raw resource patch.
 *
 * The companion patch is tagged with category "Internal" and automatically linked to the
 * user-facing bytecode patch via [dependsOn]. Metadata generation in [PatchListGenerator]
 * filters internal companion patches out of user-facing patch listings.
 */
class CompositePatchBuilder(
    val name: String,
    val description: String,
    val default: Boolean = true,
) {
    private var categoryName: String? = null
    private val compatibilities = mutableListOf<Compatibility>()
    private val dependencies = mutableListOf<Patch<*>>()
    private val extensionPaths = mutableListOf<String>()
    private val options = mutableListOf<Option<Any?>>()

    private var resourceBlock: (ResourcePatchContext.() -> Unit)? = null
    private var rawResourceBlock: (ResourcePatchContext.() -> Unit)? = null
    private var bytecodeBlock: (BytecodePatchContext.() -> Unit)? = null

    fun category(name: String) {
        categoryName = name
    }

    fun compatibleWith(vararg compatibility: Compatibility) {
        compatibilities.addAll(compatibility)
    }

    fun dependsOn(vararg patch: Patch<*>) {
        dependencies.addAll(patch)
    }

    fun extendWith(path: String) {
        extensionPaths.add(path)
    }

    @Suppress("UNCHECKED_CAST")
    fun <T> registerOption(option: Option<T>): Option<T> {
        options.add(option as Option<Any?>)
        return option
    }

    operator fun <T> invoke(option: Option<T>): Option<T> = registerOption(option)
    fun booleanOption(
        key: String,
        default: Boolean = false,
        values: Map<String, Boolean>? = null,
        title: String,
        description: String? = null,
        required: Boolean = false,
    ): Option<Boolean> {
        val opt = app.morphe.patcher.patch.booleanOption(
            key = key,
            default = default,
            values = values,
            title = title,
            description = description,
            required = required,
        )
        registerOption(opt)
        return opt
    }

    fun resources(block: ResourcePatchContext.() -> Unit) {
        check(rawResourceBlock == null) { "Cannot register both resources and rawResources in the same composite patch" }
        resourceBlock = block
    }

    fun rawResources(block: ResourcePatchContext.() -> Unit) {
        check(resourceBlock == null) { "Cannot register both resources and rawResources in the same composite patch" }
        rawResourceBlock = block
    }

    fun bytecode(block: BytecodePatchContext.() -> Unit) {
        bytecodeBlock = block
    }

    fun execute(block: BytecodePatchContext.() -> Unit) {
        bytecode(block)
    }

    fun build(): BytecodePatch {
        val companionPatch: Patch<*>? = when {
            resourceBlock != null -> resourcePatch(
                name = "$name (Internal Resource)",
                description = "Internal resource layer for $name.",
                default = default,
            ) {
                category("Internal")
                if (compatibilities.isNotEmpty()) {
                    compatibleWith(*compatibilities.toTypedArray())
                }
                execute(resourceBlock!!)
            }
            rawResourceBlock != null -> rawResourcePatch(
                name = "$name (Internal Raw Resource)",
                description = "Internal raw resource layer for $name.",
                default = default,
            ) {
                category("Internal")
                if (compatibilities.isNotEmpty()) {
                    compatibleWith(*compatibilities.toTypedArray())
                }
                execute(rawResourceBlock!!)
            }
            else -> null
        }

        return bytecodePatch(
            name = name,
            description = description,
            default = default,
        ) {
            categoryName?.let { category(it) }
            if (compatibilities.isNotEmpty()) {
                compatibleWith(*compatibilities.toTypedArray())
            }
            if (dependencies.isNotEmpty()) {
                dependsOn(*dependencies.toTypedArray())
            }
            companionPatch?.let { dependsOn(it) }
            for (ext in extensionPaths) {
                extendWith(ext)
            }
            for (opt in options) {
                opt.invoke()
            }
            bytecodeBlock?.let { execute(it) }
        }
    }
}

/**
 * Creates a composite feature patch consisting of a user-facing bytecode patch and an
 * internal companion resource/raw resource patch.
 */
fun compositeFeaturePatch(
    name: String,
    description: String,
    default: Boolean = true,
    block: CompositePatchBuilder.() -> Unit,
): BytecodePatch = CompositePatchBuilder(name, description, default).apply(block).build()
