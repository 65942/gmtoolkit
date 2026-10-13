-- GMToolKit: spawn menu browser for game content the GMToolKit app installed.
-- mount.cfg content only gets the shared "GAME" path, so the app writes each game's model list to
-- data/gmtoolkit/<id>.txt (first line: the game's name; maps-only games have just that line). This file is managed by the app; edits are overwritten.

-- Games GMod mounted itself (owned on Steam); the real engine function, before the override below.
local nativeIsMounted = IsMounted

-- Games installed by the app (one data file each). GMod's own spawnlists (CS:S, DoD:S, TF2, L4D, ...) only show
-- when IsMounted(<game>) is true, which mount.cfg content never is, so count these as mounted too.
local installed = {}
for _, f in ipairs(file.Find("gmtoolkit/*.txt", "DATA")) do installed[string.StripExtension(f)] = true end
function IsMounted(name)
	return nativeIsMounted(name) or installed[name] == true
end

local function games()
	local list = {}
	for _, f in ipairs(file.Find("gmtoolkit/*.txt", "DATA")) do
		local lines = string.Explode("\n", file.Read("gmtoolkit/" .. f, "DATA") or "")
		local name = table.remove(lines, 1)
		local id = string.StripExtension(f)
		-- Owned and mounted by GMod itself: its own Games browser covers it. Maps-only games have no models to browse.
		if name and name ~= "" and #lines > 0 and lines[1] ~= "" and not nativeIsMounted(id) then
			table.insert(list, { id = id, name = name, models = lines })
		end
	end
	table.SortByMember(list, "name", true)
	return list
end

-- Adds a folder tree for a list of model paths; icons are only created when a folder is opened.
local function addFolders(pnlContent, parent, models)
	local tree = {}
	for _, m in ipairs(models) do
		if m ~= "" then
			local node = tree
			local parts = string.Explode("/", m)
			for i = 2, #parts - 1 do -- skip the leading "models"
				node[parts[i]] = node[parts[i]] or { files = {} }
				node = node[parts[i]]
			end
			node.files = node.files or {}
			table.insert(node.files, m)
		end
	end

	local function build(treeNode, data)
		local names = {}
		for k in pairs(data) do if k ~= "files" then table.insert(names, k) end end
		table.sort(names)
		for _, name in ipairs(names) do
			local child = data[name]
			local n = treeNode:AddNode(name, "icon16/folder.png")
			n.DoClick = function(self)
				if not self.PropPanel then
					self.PropPanel = vgui.Create("ContentContainer", pnlContent)
					self.PropPanel:SetVisible(false)
					self.PropPanel:SetTriggerSpawnlistChange(false)
					for _, m in ipairs(child.files or {}) do
						spawnmenu.CreateContentIcon("model", self.PropPanel, { model = m })
					end
				end
				pnlContent:SwitchPanel(self.PropPanel)
			end
			build(n, child)
		end
	end
	build(parent, tree)
end

-- Sandbox's own "Games" folder under Browse, if it exists yet.
local function gamesFolder(browseNode)
	if not IsValid(browseNode) then return end
	local label = language.GetPhrase("spawnmenu.category.games")
	for _, child in ipairs(browseNode:GetChildNodes()) do
		local text = child:GetText()
		if text == label or text == "#spawnmenu.category.games" then return child end
	end
end

hook.Add("PopulateContent", "GMToolKit", function(pnlContent, tree, browseNode)
	local list = games()
	if #list == 0 then return end
	-- Hook order isn't guaranteed, so wait a frame for Sandbox to add its Games folder; next to the
	-- owned games is where players look. Fall back to our own folder if it isn't there.
	timer.Simple(0, function()
		if not IsValid(tree) then return end
		local root = gamesFolder(browseNode) or tree:AddNode("GMToolKit", "icon16/bricks.png")
		for _, g in ipairs(list) do
			local icon = "games/16/" .. g.id .. ".png"
			local node = root:AddNode(g.name, file.Exists("materials/" .. icon, "GAME") and icon or "icon16/folder_database.png")
			addFolders(pnlContent, node, g.models)
		end
	end)
end)

-- Games GMod ships a hand-made spawnlist for (settings/spawnlist files with "needsapp").
local function builtinLists()
	local apps = {}
	for _, dir in ipairs({ "settings/spawnlist/", "settings/spawnlist_default/" }) do
		for _, f in ipairs(file.Find(dir .. "*.txt", "MOD")) do
			local t = util.KeyValuesToTable(file.Read(dir .. f, "MOD") or "")
			if t and t.needsapp and t.needsapp ~= "" then apps[t.needsapp] = true end
		end
	end
	return apps
end

-- Models that aren't useful as props: first-person weapons and arms, gibs, skybox copies, animation-only and effect models.
local function useful(m)
	local name = string.GetFileFromFilename(m)
	return not (name:find("^v_") or name:find("^c_") or m:find("/arms/") or m:find("gib") or m:find("skybox")
		or m:find("anim") or m:find("/editor/") or m:find("^models/effects/") or name:find("^error"))
end

-- "props_farm" -> "Props Farm"
local function prettify(folder)
	return (folder:gsub("_", " "):gsub("(%a)([%w']*)", function(a, b) return a:upper() .. b end))
end

-- Games without a built-in spawnlist get one generated from their model list: one list per game, with a
-- sub-list per top-level model folder. IDs start high to stay clear of the user's own spawnlists.
hook.Add("PopulatePropMenu", "GMToolKit", function()
	local builtin = builtinLists()
	local id = 770000
	for _, g in ipairs(games()) do
		if not builtin[g.id] then
			local categories = {}
			for _, m in ipairs(g.models) do
				if m ~= "" and useful(m) then
					local parts = string.Explode("/", m)
					local folder = #parts > 2 and parts[2] or "misc"
					categories[folder] = categories[folder] or {}
					table.insert(categories[folder], { type = "model", model = m })
				end
			end
			local folders = table.GetKeys(categories)
			if #folders > 0 then
				table.sort(folders)
				local icon = "games/16/" .. g.id .. ".png"
				icon = file.Exists("materials/" .. icon, "GAME") and icon or "icon16/folder_database.png"
				local parent = id
				spawnmenu.AddPropCategory("gmtoolkit_" .. g.id, g.name, {}, icon, parent, 0, g.id)
				for _, folder in ipairs(folders) do
					id = id + 1
					spawnmenu.AddPropCategory("gmtoolkit_" .. g.id .. "_" .. folder, prettify(folder), categories[folder], "icon16/page.png", id, parent, g.id)
				end
				id = id + 1
			end
		end
	end
end)

-- A game installed by the app that is now also installed (and mounted) through Steam is loaded twice
-- until the app is opened again and removes its copy.
hook.Add("InitPostEntity", "GMToolKit", function()
	for _, f in ipairs(file.Find("gmtoolkit/*.txt", "DATA")) do
		local id = string.StripExtension(f)
		if nativeIsMounted(id) then
			local name = string.Explode("\n", file.Read("gmtoolkit/" .. f, "DATA") or "")[1] or id
			notification.AddLegacy(name .. " is now installed through Steam. Open GMToolKit to remove its duplicate copy and free up space.", NOTIFY_HINT, 12)
		end
	end
end)
