package com.myron.lrccompanion.ui

import androidx.activity.compose.BackHandler
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import com.myron.lrccompanion.AppViewModel
import com.myron.lrccompanion.Screen

@Composable
fun AppRoot(vm: AppViewModel) {
    val snackbar = remember { SnackbarHostState() }

    val msg = vm.message
    LaunchedEffect(msg) {
        if (msg != null) {
            snackbar.showSnackbar(msg)
            vm.consumeMessage()
        }
    }

    BackHandler(enabled = vm.stack.size > 1 || vm.folderStack.size > 1) { vm.back() }

    when (vm.stack.last()) {
        Screen.Browser -> BrowserScreen(vm, snackbar)
        Screen.Editor -> vm.editor?.let { EditorScreen(vm, it, snackbar) }
        Screen.Search -> vm.search?.let { SearchScreen(vm, it, snackbar) }
        Screen.Preview -> vm.selectedResult?.let { PreviewScreen(vm, it, snackbar) }
        Screen.RomajiTool -> RomajiScreen(vm, snackbar)
    }

    if (vm.showDiscardDialog) {
        AlertDialog(
            onDismissRequest = { vm.showDiscardDialog = false },
            title = { Text("Discard changes?") },
            text = { Text("Your tag or lyrics edits for this song haven't been saved.") },
            confirmButton = { TextButton(onClick = { vm.closeEditor() }) { Text("Discard") } },
            dismissButton = { TextButton(onClick = { vm.showDiscardDialog = false }) { Text("Keep editing") } },
        )
    }
}
