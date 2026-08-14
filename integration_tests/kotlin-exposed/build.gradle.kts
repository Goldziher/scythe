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
    implementation("org.postgresql:postgresql:42.7.12")
    implementation("org.jetbrains.exposed:exposed-core:0.61.0")
    implementation("org.jetbrains.exposed:exposed-jdbc:0.61.0")
    implementation("org.jetbrains.exposed:exposed-dao:0.61.0")
    // exposed-java-time supplies the Java*ColumnType classes the generated
    // queries.kt imports for date/time columns; exposed-core alone does not.
    implementation("org.jetbrains.exposed:exposed-java-time:0.61.0")
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
