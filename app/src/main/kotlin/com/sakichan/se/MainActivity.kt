package com.sakichan.se

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.Modifier
import com.sakichan.se.ui.theme.SakichanTheme
import io.github.takahashirinta.kanesumi.structure.MetroShell

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            SakichanTheme {
                MetroShell(
                    content = {
                        Box(modifier = Modifier.fillMaxSize())
                    },
                )
            }
        }
    }
}
