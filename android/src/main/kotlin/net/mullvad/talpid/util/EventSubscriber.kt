package net.mullvad.talpid.util

// Manages listeners interested in receiving events of type T
//
// The listeners subscribe using an ID object. This ID is used later on for unsubscribing. The only
// requirement is that the object uses the default implementation of the `hashCode` and `equals`
// methods inherited from `Any` (or `Object` in Java).
//
// If the ID object class (or any of its super-classes) overrides `hashCode` or `equals`,
// unsubscribe might not work correctly.
abstract class EventSubscriber<T>(initialValue: T) {
    private val listeners = HashMap<Any, (T) -> Unit>()

    var latestEvent = initialValue
        private set

    fun subscribe(id: Any, listener: (T) -> Unit) {
        synchronized(this) {
            listeners.put(id, listener)
            listener(latestEvent)
        }
    }

    fun hasListeners(): Boolean {
        synchronized(this) {
            return !listeners.isEmpty()
        }
    }

    fun unsubscribe(id: Any) {
        synchronized(this) {
            listeners.remove(id)
        }
    }

    fun unsubscribeAll() {
        synchronized(this) {
            listeners.clear()
        }
    }

    protected fun notifyListeners(event: T) {
        synchronized(this) {
            latestEvent = event

            for (listener in listeners.values) {
                listener(event)
            }
        }
    }
}
