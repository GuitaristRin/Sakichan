package com.sakichan.se.core.model

sealed interface StreamEvent {
    data class Token(val text: String) : StreamEvent
    data class ReasoningToken(val text: String) : StreamEvent
    data class ToolCall(val calls: List<ToolCall>) : StreamEvent
    data object Done : StreamEvent
    data class Error(val text: String) : StreamEvent
}

sealed interface PipelineEvent {
    data class Token(val text: String) : PipelineEvent
    data class ReasoningToken(val text: String) : PipelineEvent
    data class MemoriesInjected(val count: Int) : PipelineEvent
    data object NoMemoriesFound : PipelineEvent
    data class AnalyseImage(val text: String) : PipelineEvent
    data class Done(val result: FinalResult) : PipelineEvent
    data class Error(val text: String) : PipelineEvent
}
