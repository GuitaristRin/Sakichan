pluginManagement {
    repositories {
        google {
            content {
                includeGroupByRegex("com\\.android.*")
                includeGroupByRegex("com\\.google.*")
                includeGroupByRegex("androidx.*")
            }
        }
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "Sakichan"

include(":app")

includeBuild("../Kanesumi") {
    dependencySubstitution {
        substitute(module("io.github.takahashirinta:kanesumi-core")).using(project(":kanesumi-core"))
        substitute(module("io.github.takahashirinta:kanesumi-anim")).using(project(":kanesumi-anim"))
        substitute(module("io.github.takahashirinta:kanesumi-controls")).using(project(":kanesumi-controls"))
        substitute(module("io.github.takahashirinta:kanesumi-structure")).using(project(":kanesumi-structure"))
    }
}
