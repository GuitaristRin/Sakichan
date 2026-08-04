package com.sakichan.se.core.model

import com.sakichan.se.core.session.SessionContext
import com.sakichan.se.data.network.OpencodeClient

/**
 * Session 树:机器 -> 项目目录 -> session(BUILD.md §5 + 用户架构决策)。
 *
 * 一台机器(Machine)下多个项目(OcProject,含 worktree 目录),每个项目下若干
 * session。树由 ConnectionManager 的 projects + 按项目拉的 sessions 组合而成,
 * 供抽屉 UI 展示与切换。
 *
 * 层级 id 约定:「机器id-项目目录名-sessionid」,与用户确认的架构一致。
 */

/** 抽屉里的一棵树。根是机器。 */
data class SessionTree(
    val machine: Machine,
    val projects: List<ProjectNode> = emptyList(),
)

/** 项目节点:目录名 + 该目录下的 sessions。 */
data class ProjectNode(
    val project: OcProject,
    val sessions: List<OcSession> = emptyList(),
)

/**
 * 拉取一棵完整的树:当前机器的项目列表 + 每个项目的 sessions。
 * sessions 按 OcSession.projectID 归类到对应 ProjectNode。
 */
suspend fun OpencodeClient.buildSessionTree(
    machine: Machine,
): SessionTree {
    val projects = listProjects(machine.baseUrl)
    val sessions = listSessions(machine.baseUrl)
    val byProject = sessions.groupBy { it.projectID }
    return SessionTree(
        machine = machine,
        projects = projects.map { p ->
            ProjectNode(
                project = p,
                sessions = byProject[p.id] ?: emptyList(),
            )
        },
    )
}

/**
 * 打开一个 session:按树里定位的机器 url + session id 建立上下文。
 * 返回新的 SessionContext,供 ChatViewModel 切换会话用。
 */
suspend fun OpencodeClient.openSessionContext(
    machine: Machine,
    sessionId: String,
    systemPrompt: String,
): SessionContext {
    val session = getSession(machine.baseUrl, sessionId)
    return SessionContext(
        sessionId = session.id,
        modelId = "deepseek-v4-flash",
        systemPrompt = systemPrompt,
    ).also { ctx ->
        ctx.setDepthOrder(0, 0)
    }
}
