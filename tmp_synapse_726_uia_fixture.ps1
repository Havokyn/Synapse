param(
    [Parameter(Mandatory = $true)]
    [string]$StatePath
)

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$script:buttonCount = 0
$script:offscreenCount = 0
$script:comboDroppedDown = $false
$script:lastEvent = "started"

$form = New-Object System.Windows.Forms.Form
$form.Text = "Synapse 726 UIA Pattern FSV"
$form.Name = "synapse726Form"
$form.StartPosition = "Manual"
$form.Location = New-Object System.Drawing.Point(-500, 180)
$form.Size = New-Object System.Drawing.Size(1000, 660)
$form.TopMost = $true

$font = New-Object System.Drawing.Font("Segoe UI", 11)
$form.Font = $font

$stateLabel = New-Object System.Windows.Forms.Label
$stateLabel.Name = "stateLabel726"
$stateLabel.AccessibleName = "State Label 726"
$stateLabel.AutoSize = $true
$stateLabel.Location = New-Object System.Drawing.Point(560, 20)
$stateLabel.Text = "Button Count: 0"
$form.Controls.Add($stateLabel)

$button = New-Object System.Windows.Forms.Button
$button.Name = "invokeButton726"
$button.AccessibleName = "Invoke Button 726"
$button.Text = "Invoke Button 726"
$button.Location = New-Object System.Drawing.Point(560, 60)
$button.Size = New-Object System.Drawing.Size(220, 42)
$form.Controls.Add($button)

$checkbox = New-Object System.Windows.Forms.CheckBox
$checkbox.Name = "toggleCheckbox726"
$checkbox.AccessibleName = "Toggle Checkbox 726"
$checkbox.Text = "Toggle Checkbox 726"
$checkbox.Location = New-Object System.Drawing.Point(560, 120)
$checkbox.Size = New-Object System.Drawing.Size(260, 38)
$form.Controls.Add($checkbox)

$selectionLabel = New-Object System.Windows.Forms.Label
$selectionLabel.Name = "selectionLabel726"
$selectionLabel.AccessibleName = "Selection Label 726"
$selectionLabel.AutoSize = $true
$selectionLabel.Location = New-Object System.Drawing.Point(560, 170)
$selectionLabel.Text = "Selected Item: none"
$form.Controls.Add($selectionLabel)

$listBox = New-Object System.Windows.Forms.ListBox
$listBox.Name = "selectionList726"
$listBox.AccessibleName = "Selection List 726"
$listBox.Location = New-Object System.Drawing.Point(560, 205)
$listBox.Size = New-Object System.Drawing.Size(220, 95)
[void]$listBox.Items.Add("Alpha 726")
[void]$listBox.Items.Add("Beta 726")
[void]$listBox.Items.Add("Gamma 726")
$form.Controls.Add($listBox)

$combo = New-Object System.Windows.Forms.ComboBox
$combo.Name = "combo726"
$combo.AccessibleName = "Combo 726"
$combo.DropDownStyle = [System.Windows.Forms.ComboBoxStyle]::DropDownList
$combo.Location = New-Object System.Drawing.Point(560, 320)
$combo.Size = New-Object System.Drawing.Size(220, 38)
[void]$combo.Items.Add("Red 726")
[void]$combo.Items.Add("Green 726")
[void]$combo.Items.Add("Blue 726")
$combo.SelectedIndex = 0
$form.Controls.Add($combo)

$treeView = New-Object System.Windows.Forms.TreeView
$treeView.Name = "expandTree726"
$treeView.AccessibleName = "Expand Tree 726"
$treeView.Location = New-Object System.Drawing.Point(560, 365)
$treeView.Size = New-Object System.Drawing.Size(260, 92)
$treeRoot = New-Object System.Windows.Forms.TreeNode("Expandable Parent 726")
[void]$treeRoot.Nodes.Add("Child 726")
[void]$treeView.Nodes.Add($treeRoot)
$form.Controls.Add($treeView)

$unsupportedLabel = New-Object System.Windows.Forms.Label
$unsupportedLabel.Name = "unsupportedLabel726"
$unsupportedLabel.AccessibleName = "Unsupported Label 726"
$unsupportedLabel.AutoSize = $true
$unsupportedLabel.Location = New-Object System.Drawing.Point(560, 470)
$unsupportedLabel.Text = "Unsupported Label 726"
$form.Controls.Add($unsupportedLabel)

$removeStaleButton = New-Object System.Windows.Forms.Button
$removeStaleButton.Name = "removeStaleButton726"
$removeStaleButton.AccessibleName = "Remove Stale Target 726"
$removeStaleButton.Text = "Remove Stale Target 726"
$removeStaleButton.Location = New-Object System.Drawing.Point(560, 515)
$removeStaleButton.Size = New-Object System.Drawing.Size(240, 42)
$form.Controls.Add($removeStaleButton)

$script:staleButton = New-Object System.Windows.Forms.Button
$script:staleButton.Name = "staleTargetButton726"
$script:staleButton.AccessibleName = "Stale Target Button 726"
$script:staleButton.Text = "Stale Target Button 726"
$script:staleButton.Location = New-Object System.Drawing.Point(560, 570)
$script:staleButton.Size = New-Object System.Drawing.Size(240, 42)
$form.Controls.Add($script:staleButton)

$offscreenButton = New-Object System.Windows.Forms.Button
$offscreenButton.Name = "offscreenButton726"
$offscreenButton.AccessibleName = "Offscreen Button 726"
$offscreenButton.Text = "Offscreen Button 726"
$offscreenButton.Location = New-Object System.Drawing.Point(20, 60)
$offscreenButton.Size = New-Object System.Drawing.Size(220, 42)
$form.Controls.Add($offscreenButton)

function Write-Synapse726State {
    $selected = if ($listBox.SelectedItem) { [string]$listBox.SelectedItem } else { "none" }
    $stalePresent = ($null -ne $script:staleButton) -and
        (-not $script:staleButton.IsDisposed) -and
        $form.Controls.Contains($script:staleButton)
    $payload = [ordered]@{
        button_count = $script:buttonCount
        checkbox_checked = [bool]$checkbox.Checked
        selected_item = $selected
        combo_dropped_down = [bool]$script:comboDroppedDown
        tree_expanded = [bool]$treeRoot.IsExpanded
        offscreen_count = $script:offscreenCount
        stale_target_present = [bool]$stalePresent
        last_event = $script:lastEvent
        updated_at = (Get-Date).ToUniversalTime().ToString("o")
    }
    $json = $payload | ConvertTo-Json -Depth 4
    Set-Content -LiteralPath $StatePath -Value $json -Encoding UTF8
}

$button.Add_Click({
    $script:buttonCount += 1
    $script:lastEvent = "invoke_button"
    $stateLabel.Text = "Button Count: $script:buttonCount"
    Write-Synapse726State
})

$checkbox.Add_CheckedChanged({
    $script:lastEvent = "toggle_checkbox"
    Write-Synapse726State
})

$listBox.Add_SelectedIndexChanged({
    $script:lastEvent = "select_list_item"
    $selectionLabel.Text = "Selected Item: $($listBox.SelectedItem)"
    Write-Synapse726State
})

$combo.Add_DropDown({
    $script:comboDroppedDown = $true
    $script:lastEvent = "combo_dropdown"
    Write-Synapse726State
})

$combo.Add_DropDownClosed({
    $script:comboDroppedDown = $false
    $script:lastEvent = "combo_dropdown_closed"
    Write-Synapse726State
})

$treeView.Add_AfterExpand({
    $script:lastEvent = "tree_expand"
    Write-Synapse726State
})

$treeView.Add_AfterCollapse({
    $script:lastEvent = "tree_collapse"
    Write-Synapse726State
})

$offscreenButton.Add_Click({
    $script:offscreenCount += 1
    $script:lastEvent = "offscreen_button"
    Write-Synapse726State
})

$removeStaleButton.Add_Click({
    if (($null -ne $script:staleButton) -and $form.Controls.Contains($script:staleButton)) {
        $form.Controls.Remove($script:staleButton)
        $script:staleButton.Dispose()
    }
    $script:lastEvent = "remove_stale_target"
    Write-Synapse726State
})

$form.Add_Shown({
    Write-Synapse726State
    $stateTimer.Start()
    $form.Activate()
})

$stateTimer = New-Object System.Windows.Forms.Timer
$stateTimer.Interval = 250
$stateTimer.Add_Tick({
    Write-Synapse726State
})

$form.Add_FormClosed({
    $stateTimer.Stop()
    $stateTimer.Dispose()
})

[System.Windows.Forms.Application]::EnableVisualStyles()
[void]$form.ShowDialog()
