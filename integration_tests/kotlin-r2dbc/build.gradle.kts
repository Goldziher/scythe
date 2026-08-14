plugins {
    kotlin("jvm") version "2.1.20"
    application
}

group = "com.scythe"
version = "1.0-SNAPSHOT"

repositories {
    mavenCentral()
}

dependencies {
    implementation("org.postgresql:r2dbc-postgresql:1.0.7.RELEASE")
    // The generated Queries.kt calls kotlinx.coroutines.reactive.awaitFirst /
    // awaitFirstOrNull / asFlow on the Mono/Flux the r2dbc driver returns --
    // see kotlin_r2dbc.rs's non-extension-function code path, which this
    // project's scythe.toml selects by leaving extension_functions unset.
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-reactive:1.9.0")
    implementation("io.projectreactor:reactor-core:3.7.4")
    // The blocking JDBC driver as well, and not by accident: R2DBC has no way
    // to run a multi-statement DDL script, so the harness sets the schema up
    // over java.sql.DriverManager before any reactive code runs. Without this
    // the project compiles and then dies at run time with "No suitable driver
    // found for jdbc:...".
    implementation("org.postgresql:postgresql:42.7.12")
}

application {
    mainClass.set("IntegrationTestKt")
}

java {
    sourceCompatibility = JavaVersion.VERSION_21
    targetCompatibility = JavaVersion.VERSION_21
}

tasks.withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_21)
    }
}
