<?php

namespace Modules\VMSAcars\Http\Errors;

use App\Exceptions\AbstractHttpException;
use App\Models\Aircraft;

class VmsAcarsUpgradeRequired extends AbstractHttpException
{
    public const MESSAGE = 'Please upgrade your vmsACARS version';

    public function __construct() {
        parent::__construct(
            400,
            static::MESSAGE
        );
    }

    /**
     * Return the RFC 7807 error type (without the URL root)
     */
    public function getErrorType(): string
    {
        return 'vmsacars-upgrade-required';
    }

    /**
     * Get the detailed error string
     */
    public function getErrorDetails(): string
    {
        return 'The minimum vmsACARS version required is '.config('vmsacars.minimum_acars_version');
    }

    /**
     * Return an array with the error details, merged with the RFC7807 response
     */
    public function getErrorMetadata(): array
    {
        return [
            'minimum_version' => config('vmsacars.minimum_acars_version'),
        ];
    }
}
