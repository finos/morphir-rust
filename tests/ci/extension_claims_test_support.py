"""Object mothers for guest claims, independent of descriptor construction."""


def claims_for_extension(extension, version):
    capabilities = {}
    if "languages" in extension:
        capabilities["frontend"] = {
            "languages": [{"id": item["id"], "fileExtensions": item["file_extensions"]}
                          for item in extension["languages"]],
            "irVersions": extension["ir_versions"], "compile": True,
            "incremental": extension.get("incremental", False), "fragments": False,
        }
    if extension.get("targets"):
        capabilities["backend"] = {"targets": extension["targets"],
                                   "irVersions": extension["ir_versions"], "generate": True}
    if extension.get("workspace_discovery"):
        capabilities["workspace"] = {"discover": True, "protocolVersions": ["0.1.0-draft.1"]}
    return {
        "claimsVersion": "0.1.0-draft.2", "protocolVersions": extension["mep_versions"],
        "extension": {"id": extension["extension_id"], "name": extension.get("name", "Fixture"),
                      "version": version, "types": list(capabilities)},
        "capabilities": capabilities,
    }
