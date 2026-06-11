pipeline {
    agent {
        docker {
            image 'rust:1.75'
            args '-v /cache/cargo:/root/.cargo'
        }
    }

    environment {
        CARGO_HOME = '/root/.cargo'
        RUST_BACKTRACE = 'full'
    }

    options {
        timeout(time: 1, unit: 'HOURS')
        retry(2)
        timestamps()
        disableConcurrentBuilds()
    }

    triggers {
        cron('H 2 * * *')
        pollSCM('H/15 * * * *')
    }

    parameters {
        choice(name: 'RUST_VERSION', choices: ['1.75', 'stable', 'nightly'], description: 'rust toolchain')
        booleanParam(name: 'PUBLISH', defaultValue: false, description: 'publish to crates.io')
    }

    tools {
        rust 'rust-stable'
    }

    libraries {
        lib('shared-ci@main')
    }

    stages {
        stage('lint') {
            when {
                anyOf {
                    branch 'main'
                    branch 'release/*'
                }
            }
            steps {
                sh 'cargo fmt --check'
                sh 'cargo clippy --all-targets -- -D warnings'
            }
        }
        stage('test') {
            parallel {
                stage('unit') {
                    steps {
                        sh 'cargo test --workspace --release --lib'
                    }
                }
                stage('integration') {
                    steps {
                        sh 'cargo test --workspace --release --test integration'
                    }
                }
            }
        }
        stage('build') {
            steps {
                sh 'cargo build --release --workspace'
            }
            post {
                success {
                    archiveArtifacts artifacts: 'target/release/*', fingerprint: true
                }
            }
        }
        stage('publish') {
            when {
                allOf {
                    expression { params.PUBLISH }
                    tag pattern: 'v*', comparator: 'GLOB'
                }
            }
            steps {
                sh 'cargo publish --token $CARGO_REGISTRY_TOKEN'
            }
        }
    }

    post {
        always {
            junit 'target/test-results/junit.xml'
            cleanWs()
        }
        failure {
            mail to: 'team@example.com', subject: "build failed: ${env.BUILD_NUMBER}", body: 'see ${env.BUILD_URL}'
        }
        success {
            echo 'build succeeded'
        }
    }
}
