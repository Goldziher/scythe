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
    implementation("net.snowflake:snowflake-jdbc:4.0.2")
}

application {
    mainClass.set("IntegrationTestKt")
}
tasks.named<JavaExec>("run") {
    // snowflake-jdbc bundles Apache Arrow, which reflectively accesses
    // java.nio.Buffer's internal address field for off-heap memory access.
    // The JDK 9+ module system blocks that by default, so without this the
    // driver fails with InaccessibleObjectException as soon as it tries to
    // decode an Arrow result.
    jvmArgs = listOf("--add-opens=java.base/java.nio=ALL-UNNAMED")
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
