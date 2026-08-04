package com.sakichan.se.core.error

sealed class SakichanError(message: String, cause: Throwable? = null) : Exception(message, cause) {
    data class Config(override val message: String) : SakichanError(message)
    data class Network(override val message: String, override val cause: Throwable? = null) : SakichanError(message, cause)
    data class Api(val status: Int, override val message: String) : SakichanError(message)
    data class Database(override val message: String, override val cause: Throwable? = null) : SakichanError(message, cause)
    data class Io(override val message: String, override val cause: Throwable? = null) : SakichanError(message, cause)
    data class Serialization(override val message: String, override val cause: Throwable? = null) : SakichanError(message, cause)
    data class UserInput(override val message: String) : SakichanError(message)
    data class Unknown(override val message: String) : SakichanError(message)
}
