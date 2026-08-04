package com.sakichan.se.data.discovery

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import com.sakichan.se.core.model.Machine
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.net.InetAddress
import java.security.MessageDigest

/**
 * 用 Android NsdManager 扫描局域网 mDNS 服务。opencode 用 bonjour-service 发布:
 * service type = `_http._tcp`,name = `opencode-{port}`,host = `opencode.local`,
 * txt = {path: "/"}。所以扫描 `_http._tcp` 后按 name 前缀 `opencode-` 过滤即可。
 *
 * 跨平台:opencode 在 Windows/Linux/macOS 上都走同一 bonjour-service 发布,Android
 * NsdManager 只认标准 mDNS 协议,不关心服务端 OS。多机器各自发布,扫描全部可见。
 *
 * 用法:[scan] 返回 Flow,每次发现/丢失服务时 emit 当前完整列表;调用方 collect 渲染。
 * 扫描结束自动 stop。手动输入的机器不经过这里。
 */
class DiscoveryService(private val context: Context) {

    private val nsdManager: NsdManager by lazy {
        context.getSystemService(Context.NSD_SERVICE) as NsdManager
    }

    /** 扫描 opencode 机器。Flow 取消时自动 stopScan。每次变更 emit 全量列表。 */
    fun scan(): Flow<List<Machine>> = callbackFlow {
        val found = LinkedHashMap<String, Machine>()  // id -> machine,保持稳定顺序
        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) {}
            override fun onDiscoveryStopped(serviceType: String) {}

            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
                channel.close()
            }

            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}

            override fun onServiceFound(info: NsdServiceInfo) {
                // 只关心 opencode- 前缀的服务(bonjour name = opencode-{port})
                if (!info.serviceName.startsWith(SERVICE_NAME_PREFIX)) return
                resolve(info)
            }

            override fun onServiceLost(info: NsdServiceInfo) {
                if (!info.serviceName.startsWith(SERVICE_NAME_PREFIX)) return
                val id = machineId(info.serviceName, info.host, info.port)
                found.remove(id)
                trySend(found.values.toList())
            }

            private fun resolve(info: NsdServiceInfo) {
                val resolveListener = object : NsdManager.ResolveListener {
                    override fun onServiceResolved(svc: NsdServiceInfo) {
                        val host = svc.host ?: return
                        val port = svc.port
                        val name = svc.serviceName
                        val baseUrl = "http://${host.hostAddress}:$port"
                        val id = machineId(baseUrl)
                        found[id] = Machine(
                            id = id,
                            baseUrl = baseUrl,
                            name = name,
                            host = host.hostAddress ?: host.canonicalHostName,
                            port = port,
                            source = Machine.Source.SCANNED,
                        )
                        trySend(found.values.toList())
                    }

                    override fun onResolveFailed(svc: NsdServiceInfo, errorCode: Int) {
                        // 解析失败就跳过,不影响其他机器
                    }
                }
                try {
                    nsdManager.resolveService(info, resolveListener)
                } catch (_: Exception) {
                    // 并发 resolve 限制:同时只能有一个 resolve,失败则跳过
                }
            }
        }

        try {
            nsdManager.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
        } catch (_: Exception) {
            channel.close(); return@callbackFlow
        }

        awaitClose {
            try {
                nsdManager.stopServiceDiscovery(listener)
            } catch (_: Exception) {
            }
        }
    }

    companion object {
        const val SERVICE_TYPE = "_http._tcp."
        const val SERVICE_NAME_PREFIX = "opencode-"

        /** baseUrl 去尾斜杠后取 SHA-1 前 12 位,作为机器稳定 id。 */
        fun machineId(baseUrl: String): String {
            val normalized = baseUrl.trimEnd('/')
            val md = MessageDigest.getInstance("SHA-1")
            return md.digest(normalized.toByteArray())
                .joinToString("") { "%02x".format(it) }
                .take(12)
        }

        private fun machineId(name: String, host: InetAddress?, port: Int): String {
            val addr = host?.hostAddress ?: name
            return machineId("http://$addr:$port")
        }
    }
}
