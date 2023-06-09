use crate::auth::{
    get_external_user_info, AuthError, AuthPage, AuthServiceState, AuthSession, ExternalLogin, OAuth2Client,
};
use axum::{
    extract::{Query, State},
    Extension,
};
use oauth2::{reqwest::async_http_client, AuthorizationCode, PkceCodeVerifier, TokenResponse};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub(in crate::auth) struct RequestParams {
    code: String,
    state: String,
}

/// Process the authentication redirect from the OAuth2 provider.
pub(in crate::auth) async fn page_oauth2_auth(
    State(state): State<AuthServiceState>,
    Extension(client): Extension<Arc<OAuth2Client>>,
    Query(query): Query<RequestParams>,
    mut auth_session: AuthSession,
) -> AuthPage {
    let auth_code = AuthorizationCode::new(query.code);
    let auth_csrf_state = query.state;

    // take external_login from session, thus later code don't have to care with it
    let ExternalLogin {
        pkce_code_verifier,
        csrf_state,
        target_url,
        error_url,
        remember_me,
        linked_user,
        ..
    } = match auth_session.external_login.take() {
        Some(external_login) => external_login,
        None => return state.page_error(auth_session, AuthError::MissingExternalLogin, None),
    };

    // Check for Cross Site Request Forgery
    if csrf_state != auth_csrf_state {
        log::debug!("CSRF test failed: [{csrf_state}], [{auth_csrf_state}]");
        return state.page_error(auth_session, AuthError::InvalidCSRF, error_url.as_ref());
    }

    // Exchange the code with a token.
    let token = match client
        .client
        .exchange_code(auth_code)
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_code_verifier))
        .request_async(async_http_client)
        .await
    {
        Ok(token) => token,
        Err(err) => return state.page_internal_error(auth_session, err, error_url.as_ref()),
    };

    let external_user_info = match get_external_user_info(
        client.user_info_url.url().clone(),
        &client.provider,
        token.access_token().secret(),
        &client.user_info_mapping,
        &client.extensions,
    )
    .await
    {
        Ok(external_user_info) => external_user_info,
        _ => return state.page_error(auth_session, AuthError::FailedExternalUserInfo, error_url.as_ref()),
    };
    log::info!("{:?}", external_user_info);

    if linked_user.is_some() {
        state
            .page_external_link(
                auth_session,
                &client.provider,
                &external_user_info.provider_id,
                target_url.as_ref(),
                error_url.as_ref(),
            )
            .await
    } else {
        state
            .page_external_login(
                auth_session,
                external_user_info,
                target_url.as_ref(),
                error_url.as_ref(),
                remember_me,
            )
            .await
    }
}
